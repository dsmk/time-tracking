#!/usr/bin/env bash
#
# Render the realm import file from realm-template.json.
#
# Keycloak's own `--import-realm` reads a finished JSON file; it has no
# documented, supported placeholder syntax of its own, and the `$(env:...)`
# form belongs to the third-party keycloak-config-cli, not to Keycloak. So we
# substitute here, in an init container, and keep every secret in a Kubernetes
# Secret rather than in a committed JSON file.
#
# Inputs (environment):
#   REALM                  realm name
#   OIDC_CLIENT_ID         client id OpenTodo authenticates as
#   OIDC_CLIENT_SECRET     its secret (same value as OPENTODO_OIDC_CLIENT_SECRET)
#   OPENTODO_BASE_URL      https origin of the OpenTodo server
#   GITHUB_CLIENT_ID       GitHub OAuth app client id   (optional)
#   GITHUB_CLIENT_SECRET   GitHub OAuth app secret      (optional)
#   DEV_USER / DEV_USER_EMAIL / DEV_USER_PASSWORD        (optional, kind only)
#   TEMPLATE / OUT_DIR / OUT_MODE                        (optional, defaulted)
#
# Output: /opt/keycloak/data/import/realm.json
#
# IMPORTANT: `--import-realm` only creates a realm that does not exist yet. It
# never updates one. Changing a redirect URI or a secret after the first start
# means editing the live realm (admin console, or kcadm.sh) — see
# deploy/k8s/README.md, "Changing the realm after first start".
set -euo pipefail

template=${TEMPLATE:-/realm/realm-template.json}
out_dir=${OUT_DIR:-/opt/keycloak/data/import}
out=$out_dir/realm.json

# Escape an arbitrary value so it can sit inside a JSON string literal.
json_escape() {
  local s=$1
  s=${s//\\/\\\\}
  s=${s//\"/\\\"}
  s=${s//$'\n'/\\n}
  s=${s//$'\r'/\\r}
  s=${s//$'\t'/\\t}
  printf '%s' "$s"
}

doc=$(cat "$template")

# Replace one placeholder everywhere. Bash pattern substitution is literal on
# both sides when quoted, so no escaping of the replacement is needed beyond
# the JSON escaping above.
subst() {
  local token=$1 value=$2
  doc=${doc//"$token"/"$value"}
}

realm=${REALM:?REALM is required}
client_id=${OIDC_CLIENT_ID:?OIDC_CLIENT_ID is required}
client_secret=${OIDC_CLIENT_SECRET:?OIDC_CLIENT_SECRET is required}
base_url=${OPENTODO_BASE_URL:?OPENTODO_BASE_URL is required}
base_url=${base_url%/}

case "$base_url" in
  https://*) ;;
  *) echo "render-realm: OPENTODO_BASE_URL must be https (got '$base_url')" >&2; exit 1 ;;
esac

# --- GitHub identity provider -------------------------------------------
#
# trustEmail is the setting the whole bridge turns on. OpenTodo refuses any
# identity whose ID token lacks "email_verified": true (error
# oidc_email_unverified). Keycloak only populates that claim from the brokered
# user's emailVerified flag, and only sets that flag when the provider is
# trusted. Without trustEmail every GitHub sign-in is rejected by OpenTodo.
#
# defaultScope "read:user user:email" lets Keycloak call /user/emails and pick
# the primary verified address.
identity_providers=""
if [ -n "${GITHUB_CLIENT_ID:-}" ] && [ -n "${GITHUB_CLIENT_SECRET:-}" ]; then
  identity_providers=$(cat <<EOF
{
      "alias": "github",
      "displayName": "GitHub",
      "providerId": "github",
      "enabled": true,
      "trustEmail": true,
      "storeToken": false,
      "addReadTokenRoleOnCreate": false,
      "linkOnly": false,
      "firstBrokerLoginFlowAlias": "first broker login",
      "config": {
        "clientId": "$(json_escape "$GITHUB_CLIENT_ID")",
        "clientSecret": "$(json_escape "$GITHUB_CLIENT_SECRET")",
        "defaultScope": "read:user user:email",
        "syncMode": "FORCE",
        "hideOnLoginPage": "false"
      }
    }
EOF
)
else
  echo "render-realm: GITHUB_CLIENT_ID/SECRET not set - realm will have no identity provider" >&2
fi

# --- Optional local user (kind / demo only) ------------------------------
users=""
if [ -n "${DEV_USER:-}" ] && [ -n "${DEV_USER_PASSWORD:-}" ]; then
  users=$(cat <<EOF
{
      "username": "$(json_escape "$DEV_USER")",
      "email": "$(json_escape "${DEV_USER_EMAIL:-$DEV_USER@example.test}")",
      "emailVerified": true,
      "enabled": true,
      "firstName": "Local",
      "lastName": "Tester",
      "credentials": [
        {
          "type": "password",
          "value": "$(json_escape "$DEV_USER_PASSWORD")",
          "temporary": false
        }
      ]
    }
EOF
)
  echo "render-realm: seeding local test user '${DEV_USER}'" >&2
fi

subst __REALM__ "$(json_escape "$realm")"
subst __OIDC_CLIENT_ID__ "$(json_escape "$client_id")"
subst __OIDC_CLIENT_SECRET__ "$(json_escape "$client_secret")"
subst __OPENTODO_BASE_URL__ "$(json_escape "$base_url")"
subst __IDENTITY_PROVIDERS__ "$identity_providers"
subst __USERS__ "$users"

for token in __REALM__ __OIDC_CLIENT_ID__ __OIDC_CLIENT_SECRET__ \
             __OPENTODO_BASE_URL__ __IDENTITY_PROVIDERS__ __USERS__; do
  case "$doc" in
    *"$token"*)
      echo "render-realm: placeholder $token was never substituted" >&2
      exit 1
      ;;
  esac
done

mkdir -p "$out_dir"
printf '%s\n' "$doc" > "$out"
# The file carries the client secrets. 0600 suits the pod-local emptyDir this
# normally lands in; the podman deployment passes OUT_MODE=0644 because there
# the container's own user has to read a bind-mounted file, and the 0700
# directory around it is what keeps everyone else out.
chmod "${OUT_MODE:-600}" "$out"
echo "render-realm: wrote $out ($(wc -c <"$out") bytes) for realm '$realm'" >&2
