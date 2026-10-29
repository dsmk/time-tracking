# OpenTodo on Kubernetes, with GitHub sign-in through Keycloak

A deployable OpenTodo server behind an HTTPS ingress, plus a Keycloak realm
that brokers GitHub accounts into OpenTodo's OIDC client. One set of
manifests, two overlays: a real cluster, and a throwaway `kind` cluster on
your laptop.

This directory is infrastructure for the server `tt` talks to. It does not
change `tt`, and `tt` does not require it — see
[How `tt` signs in](#how-tt-signs-in) for the part that matters to this repo.

## What this is built from

Keeping with [`docs/opentodo-compatibility.md`](../../docs/opentodo-compatibility.md),
here is where the configuration comes from and how far it has been checked:

| | |
|---|---|
| **OpenTodo settings** | Every `OPENTODO_*` variable used here appears in the upstream configuration table and the self-hosting pages: [docker](https://open-todo.app/docs/self-hosting/docker/), [single sign-on](https://open-todo.app/docs/self-hosting/sso/), [kubernetes](https://open-todo.app/docs/self-hosting/kubernetes/). |
| **Pod shape** | Mirrors what upstream's own Helm chart is documented to do: one replica, `Recreate`, one RWO volume at `/data`, UID/GID/fsGroup 65532, read-only root. |
| **Keycloak settings** | From [the reverse proxy guide](https://www.keycloak.org/server/reverseproxy) and [the container guide](https://www.keycloak.org/server/containers). |
| **Verification** | `kustomize build` is clean for both overlays. **These manifests have not been applied to a live cluster from this repository.** Nothing here is tagged "tested against a running stack". |

There is also an [official Helm chart](https://open-todo.app/docs/self-hosting/kubernetes/)
(`oci://ghcr.io/pixari/charts/opentodo`). If you would rather use it, do — it
is upstream's own and it will track their releases. These manifests exist
because the interesting half of this deployment is the Keycloak realm and the
proxy wiring, and keeping both halves in one kustomize tree makes the
relationship between them readable.

## How the pieces fit

```
  browser ──https──▶ ingress-nginx ──http──▶ opentodo       (8080)
     │                    │
     │                    └────────────────▶ keycloak       (8080)
     │                                          │
     └──https──▶ github.com ◀──token exchange───┘
                                                │
                    opentodo ──https──▶ keycloak│  discovery + token,
                              (same issuer URL) │  server to server
```

Three things have to agree, or sign-in fails in ways that are tedious to
debug:

1. **`OPENTODO_BASE_URL`** equals the address in the browser, exactly. If it
   does not, OpenTodo refuses with *"cross-site request rejected"*.
2. **`OPENTODO_OIDC_ISSUER`** equals the `issuer` field of
   `<issuer>/.well-known/openid-configuration`, character for character. For
   Keycloak that is `<KC_HOSTNAME>/realms/<realm>`.
3. **The issuer URL resolves from inside the OpenTodo pod as well as from the
   browser.** OpenTodo fetches discovery and calls the token endpoint itself.
   In production that is just public DNS; under `kind` it needs a CoreDNS
   rewrite, which `kind/bootstrap.sh` installs.

The redirect URI Keycloak is configured with is the one OpenTodo documents:

```
<OPENTODO_BASE_URL>/api/v1/auth/oidc/callback
```

and GitHub's OAuth app callback is Keycloak's broker endpoint:

```
<KC_HOSTNAME>/realms/<realm>/broker/github/endpoint
```

### The one setting the bridge turns on

OpenTodo refuses any identity whose ID token lacks `"email_verified": true`
(error `oidc_email_unverified`), and refuses to create or link an account
without it. Keycloak only puts that claim in the token when the brokered
user's `emailVerified` flag is set, and only sets that flag when the identity
provider is marked **Trust Email**.

So `"trustEmail": true` on the GitHub provider is not a convenience — without
it every GitHub sign-in is rejected at the OpenTodo end. It is set in
[`base/keycloak/render-realm.sh`](base/keycloak/render-realm.sh), together
with the `read:user user:email` scope that lets Keycloak read the primary
verified address from `/user/emails`.

Trusting GitHub's verified-email flag is a real decision: it means anyone who
can prove an email address to GitHub can present it to you. If that is too
loose, leave *Allow new accounts via SSO* off and invite people individually —
then a GitHub identity can only attach to an account you created.

## Layout

```
base/                       hostnames are placeholders; not deployable alone
  opentodo/                 Deployment, Service, PVC, Ingress
  keycloak/                 Deployment, Service, Ingress, PostgreSQL
    realm-template.json     the realm, with placeholders
    render-realm.sh         init container: template + Secrets -> import file
overlays/
  production/               real hostnames, Let's Encrypt, larger volumes
  kind/                     *.kind.test, a local CA, a seeded test user
kind/
  cluster.yaml              single node, :80 and :443 forwarded to the host
  ca-issuer.yaml            self-signed root + the ClusterIssuer that uses it
  bootstrap.sh              cluster, ingress, cert-manager, DNS, deploy
  teardown.sh
```

## Running under kind

```bash
cd deploy/k8s/kind
./bootstrap.sh
```

Needs `kind`, `kubectl` and a container runtime. It creates the cluster,
installs ingress-nginx and cert-manager, creates a local CA, adds the CoreDNS
rewrite, generates `overlays/kind/secrets.env` with random values, deploys,
and prints what is left for you. Running it again is safe.

Three steps stay manual, and `bootstrap.sh` prints all three:

1. **`/etc/hosts`.** Add `127.0.0.1 todo.kind.test keycloak.kind.test`. `kind`
   forwards the host's :80 and :443 into the cluster, so that is all the
   browser needs. `.test` is reserved by RFC 6761 and will never resolve
   anywhere real.
2. **The CA.** `bootstrap.sh` writes `kind/kind-ca.crt`. Import it into your
   browser, or accept the warning once per hostname.
3. **The owner account.** OpenTodo creates its owner through the web
   interface, and there is no documented way to do it from the command line —
   the same limitation [`testing/README.md`](../../testing/README.md) runs
   into. Open `https://todo.kind.test` and use the setup token
   `bootstrap.sh` printed. (It is needed because the request arrives through
   the ingress, not from loopback.)

By default the realm also gets a local Keycloak user, so you can walk the
whole SSO path without registering a GitHub OAuth app. To test the real
bridge, register one with the callback URL above, put the two values in
`overlays/kind/secrets.env`, and `kubectl apply -k overlays/kind`. GitHub does
not check that the hostname resolves and never connects to it itself — it
redirects your browser, which `/etc/hosts` already covers.

### Why kind needs a DNS rewrite and a CA

Both come from the same place: OpenTodo is an OIDC *client*, not just a page
in your browser. It fetches the discovery document and exchanges the code for
a token itself, from inside the pod, at the same `https://keycloak.kind.test`
the browser used.

`127.0.0.1` inside a pod is the pod, so the hostname has to resolve to the
ingress controller there instead — hence the CoreDNS `rewrite` rule. And the
certificate on that connection is signed by a CA no public trust store knows,
so the pod gets the root mounted and `SSL_CERT_FILE` pointed at it (see
[`overlays/kind/opentodo-ca-trust.yaml`](overlays/kind/opentodo-ca-trust.yaml)).
That env var *replaces* the Go trust store rather than adding to it, which is
only acceptable because this overlay disables every other outbound HTTPS call
OpenTodo would make. It is deliberately not in the production overlay.

Tear down with `./teardown.sh`.

## Running in production

Assumes a cluster with `ingress-nginx` and `cert-manager` already installed,
a default StorageClass providing ReadWriteOnce volumes that honour `fsGroup`,
and two DNS names pointing at your ingress controller.

```bash
cd deploy/k8s/overlays/production

$EDITOR stack.env                    # 1. your two hostnames
cp secrets.env.example secrets.env   # 2. fill in; it is gitignored
$EDITOR letsencrypt-clusterissuer.yaml             # 3. your email

kubectl apply -f letsencrypt-clusterissuer.yaml    # once per cluster
kubectl apply -k .
```

Hostnames live in `stack.env` and nowhere else. The overlay copies them into
the Ingress rules and TLS blocks with a kustomize `replacement`, so the URL
OpenTodo is configured with, the route that serves it, and the certificate
cannot drift apart — which is exactly the failure mode the three rules above
describe. `keycloak-admin-ingress.yaml` is the one exception: edit its host
directly if you enable it.

Then, in order:

1. **Register the GitHub OAuth app** before the first `apply`, with callback
   URL `https://<keycloak host>/realms/opentodo/broker/github/endpoint`. The
   realm import only runs once (see below), so having the credentials in
   `secrets.env` from the start saves a manual fix-up.
2. **Create the OpenTodo owner** at `https://<todo host>` using
   `OPENTODO_SETUP_TOKEN`. Do this promptly — until it exists, anyone holding
   the token can claim the instance.
3. **Link the owner, or do not.** Owners, admins, and any account with 2FA or
   a passkey are never linked to a brokered identity by matching email; they
   must link it from *Settings → Account → Security → Linked accounts*, or
   they will see `oidc_link_required`. That is upstream's design, and it is
   also your way back in if the provider breaks.
4. **Decide how people get accounts.** Leave `OPENTODO_ALLOW_SIGNUP` unset
   (the manifests do) and use *Settings → People → Single sign-on → Allow new
   accounts via SSO*. The environment variable would open password
   registration as well, and setting it locks the panel field.

### Secrets

`secretGenerator` reads `secrets.env` from disk at build time, which is fine
for a small deployment and wrong for a shared repository. The file is
gitignored. If you already run SOPS, Sealed Secrets, External Secrets or
Vault, delete the `secretGenerator` block and supply a Secret named
`stack-secrets` yourself; the key names in `secrets.env.example` are the
contract.

`OIDC_CLIENT_SECRET` is used twice — Keycloak's client definition and
OpenTodo's `OPENTODO_OIDC_CLIENT_SECRET` — and must be the same value in both.

Note that OpenTodo stores its copy at `/data/secrets/oidc.json`, and that
upstream backups **exclude** that directory. Restoring to a new host without
re-supplying this Secret leaves SSO broken.

### The database

`base/keycloak/postgres.yaml` is a single PostgreSQL instance with one volume:
enough to run the stack, and not a managed database. For anything you care
about, point `KC_DB_URL` at a real one and drop that file from
`base/kustomization.yaml`.

### The admin console

Not exposed. The realm is created from the import file, so you do not need the
console to run the stack. For one-off changes:

```bash
kubectl -n opentodo port-forward deploy/keycloak 8080:8080
# then http://localhost:8080/admin/ — but see below
```

`KC_HOSTNAME` is a full URL, so Keycloak builds absolute links from it and the
console will try to send you to the public hostname. If you want the console
reachable, the honest answer is to publish it: `keycloak-admin-ingress.yaml`
is ready for that, restricted to a source range you must set first. It is
commented out of `kustomization.yaml`.

Or do it without a browser at all:

```bash
kubectl -n opentodo exec -it deploy/keycloak -- /opt/keycloak/bin/kcadm.sh \
  config credentials --server http://localhost:8080 \
  --realm master --user admin
```

## Changing the realm after first start

**`--import-realm` creates a realm that does not exist. It never updates
one.** Editing `realm-template.json` and reapplying changes nothing on a
cluster that has already started once. This is the single most surprising
thing about this setup, so:

| What changed | What to do |
|---|---|
| OpenTodo's hostname | Update the client's redirect URI and web origin, then `OPENTODO_BASE_URL` |
| GitHub client id/secret | Update the `github` identity provider's config |
| `OIDC_CLIENT_SECRET` | Update the client's credential *and* the Secret, together |
| Anything structural | Easier to delete the realm and let the import recreate it — you lose brokered links, and people re-link on next sign-in |

With `kcadm.sh`, after `config credentials` as above:

```bash
# redirect URI
kcadm.sh update clients/$(kcadm.sh get clients -r opentodo \
  -q clientId=opentodo --fields id --format csv --noquotes) \
  -r opentodo \
  -s 'redirectUris=["https://todo.example.com/api/v1/auth/oidc/callback"]'

# GitHub credentials
kcadm.sh update identity-provider/instances/github -r opentodo \
  -s 'config.clientId=...' -s 'config.clientSecret=...'
```

If you would rather have the whole realm reconciled on every apply, that is
what [keycloak-config-cli](https://github.com/adorsys/keycloak-config-cli) is
for: run it as a post-install Job against the same JSON. It is a third-party
tool, which is why it is not wired in here.

## Optional: skip the Keycloak login page

OpenTodo's sign-in button says "GitHub" (`OPENTODO_OIDC_DISPLAY_NAME`), but
out of the box it lands on Keycloak's login page, which then offers a GitHub
button. To make it go straight to GitHub, configure the **Identity Provider
Redirector** in the realm's browser flow with `defaultProvider: github`
(*Authentication → Flows → browser → Identity Provider Redirector → ⚙*).

It is not in the realm template on purpose: it would also remove the only way
to sign in with the local test user the `kind` overlay seeds, and it hides the
Keycloak-side account chooser.

## When the provider breaks

If *Require single sign-on* is on and Keycloak is unreachable, nobody can sign
in. Two documented ways back:

- `https://<todo host>/?owner=1` gives the owner a password form regardless.
- Set `OPENTODO_OIDC_SSO_ONLY=false` on the OpenTodo Deployment and restart.
  The environment value wins over the panel and keeps winning until removed.
  The manifests leave this variable unset precisely so it stays available as
  the switch.

## How `tt` signs in

None of this changes how `tt` reaches OpenTodo. `tt` runs `ot`, and `ot`
authenticates with an API token — upstream states API tokens and the CLI keep
working even with SSO-only sign-in.

What does change: an account created through SSO has **no password**, so
`ot login <url>` with an email and password is not available to it. Create a
token in the web interface and hand it to `ot`:

```bash
ot auth set-token --profile prod https://todo.example.com
# or
export OPENTODO_URL=https://todo.example.com OPENTODO_TOKEN=...
```

`tt` stores neither, as [the README](../../README.md) says; it reads whatever
`ot` is configured with.

## Operational notes

- **Never scale OpenTodo past one replica.** It is one SQLite file with one
  writer. The Deployment is `Recreate` for the same reason, so upgrades cost a
  few seconds of downtime.
- **Upgrades** are a tag change in `base/opentodo/deployment.yaml`. The image
  is pinned (`0.16.0`); `stable` moves with every release.
- **Backups** are OpenTodo's own (`OPENTODO_BACKUP_*`, or the `/data` volume),
  and remember that `/data/secrets` is excluded from its backups.
- **Ingress annotations are load-bearing.** `proxy-buffering: off` and the
  long read timeout keep `/api/v1/sync/events` (Server-Sent Events) alive;
  without them live updates stall. `proxy-body-size` must stay above the
  attachment and import limits.
- **`OPENTODO_TRUSTED_PROXIES`** needs setting only when another proxy sits in
  front of the ingress controller (`cloudflare` is a recognised keyword). The
  defaults already trust a pod network.
- **Keycloak's management port (9000)** carries `/health` and `/metrics` and
  is deliberately not in any Ingress.
- **Keycloak starts slowly** here: `start` without `--optimized` augments the
  server on every boot. Build an optimized image with `kc.sh build` if that
  bothers you, and add `--optimized` to the args.

## Still to verify against a live cluster

In the spirit of [§11 of the compatibility
doc](../../docs/opentodo-compatibility.md#11-still-to-verify-against-a-live-instance):

1. That `GET /healthz` is the right probe path for the container image. It is
   what this repo's own test harness polls, but upstream documents the
   `opentodo healthcheck` command rather than an HTTP path.
2. That the OpenTodo image tolerates `SSL_CERT_FILE` (it is a Go binary, so it
   should, but this is inference).
3. That Keycloak's `--import-realm` accepts the rendered realm exactly as
   written — in particular the `github` provider block and the client's
   `pkce.code.challenge.method` attribute.
4. That a brokered GitHub identity arrives at OpenTodo with
   `email_verified: true` and is accepted.
5. End-to-end: GitHub sign-in creating or linking an OpenTodo account, then an
   API token from that account driving `ot` and `tt`.
