# tt — terminal time tracking for OpenTodo

`tt` is a small Rust CLI that makes [OpenTodo](https://open-todo.app) time
tracking fast from a terminal. It starts and stops timers, lists entries, and
produces weekly and date-range reports.

**OpenTodo owns the data.** `tt` is a front end for the documented `ot`
command-line client: it runs `ot` with an argument vector, reads its documented
`--json` output, and keeps no time-entry storage of its own. Everything `tt`
writes shows up in the OpenTodo web interface and on your other devices,
because it is the same data.

Built against `SPEC.md`. What `ot` is documented to do, and what that means for
`tt`, is recorded in [`docs/opentodo-compatibility.md`](docs/opentodo-compatibility.md).

## Honest limitations, up front

| | |
|---|---|
| **Offline** | Not supported, at all. The `ot` client is documented as online-only and keeps no local copy, so `tt` cannot start, stop, or list entries without a reachable server. It fails with exit 4 rather than queueing anything. |
| **Sync status** | `ot` exposes none. `tt sync status` says so and reports server reachability separately; it never claims "synchronized". `tt sync` exits 6 — OpenTodo documents no sync command for tasks or time. |
| **Reports** | OpenTodo has no CLI reporting command, so `tt` computes reports from `ot time list` data and tells you when a result may be incomplete. |
| **Verification** | The adapter is built from OpenTodo's documentation and tested against a mock `ot`. It has **not** been run against a live OpenTodo server; [`docs/live-check.md`](docs/live-check.md) is the checklist for doing that. |

## Requirements

- Rust 1.85 or later (edition 2024)
- The OpenTodo CLI, version 0.9.0 or later, signed in:

  ```bash
  go install github.com/OpenTodoApp/opentodo/cmd/ot@latest
  ot login https://todo.example.com
  ```

  A plain `go install` leaves `ot`'s version string at `dev`, which `tt`
  cannot read as a version number — it still works, but warns on every
  command. Stamp it to silence that:

  ```bash
  go install -ldflags="-X main.version=v0.16.0" \
    github.com/OpenTodoApp/opentodo/cmd/ot@v0.16.0
  ```

  Or use the prebuilt container in [`containers/`](containers/), which does
  that for you.

`tt` stores no credentials and no server URL. Authentication, profiles and
instance selection stay with `ot` (`ot auth status`, `OPENTODO_URL`,
`OPENTODO_TOKEN`, `--profile`).

## Install

```bash
cargo install --path .
```

## Usage

### Start and stop

```bash
tt start "Design review" --task 8c2a   # the text becomes the entry's note
tt start --task 8c2a                   # no note
tt start "Design review"               # picks a task interactively, in a terminal
tt stop
tt status
tt status --json
```

OpenTodo attaches every time entry to a task, so a task is always required.
Run from a terminal without `--task` and `tt` lists your tasks and asks; in a
script, `--task` is mandatory and `tt` says so instead of guessing.

OpenTodo runs one timer at a time and ends the current entry when another
starts. `tt start` refuses when something is already running unless you pass
`--switch`, so nothing is closed by accident.

`tt stop` checks what is running before writing. If nothing is, it exits 8 and
sends nothing to OpenTodo.

### Listing entries

```bash
tt log                                      # the last 7 days
tt log --today
tt log --from 2026-10-20 --to 2026-10-27
tt log --task 8c2a
tt log --format json
```

### Reports

```bash
tt report week
tt report week --week 2026-10-27          # any date inside the week
tt report week --week-start sunday
tt report week --format json
tt report range --from 2026-10-01 --to 2026-10-31 --format csv
```

A report shows the date range, the total, daily totals, duration by task,
untasked time, and a note whenever a timer is still running or the data may be
incomplete. Text, JSON and CSV all carry that incompleteness flag.

```text
Week of 2026-10-26 to 2026-11-01 (week starts monday, America/New_York)
-----------------------------------------------------------------------
Total: 3h 30m

By day
  Mon 2026-10-26    1h 00m
  Tue 2026-10-27    2h 00m
  Wed 2026-10-28       30m
  ...

By task
    2h 00m  Design review (Work)  [8c2a]
    1h 30m  Planning (Work)  [3f1d]
```

### Sync

```bash
tt sync status
tt sync status --json
```

OpenTodo's sync indicator lives in its app, not in `ot`. `tt sync status`
reports three separate things and never conflates them: sync state (always
*unavailable*), whether the server could be reached, and whether local changes
could be pending (they cannot — `ot` has no local queue).

## Timezones and week boundaries

Day and week boundaries are `tt`'s own, computed in the configured timezone
from the absolute timestamps OpenTodo returns. Every report prints the timezone
and week start it used.

This matters because the `ot` CLI has no documented `--tz` flag, so `tt` does
not depend on how `ot` interprets `--from`/`--to`. It asks for a window one day
wider on each side and clips entries itself. An entry crossing local midnight
is split across both days, and daily totals always sum to the period total,
including across daylight-saving changes.

## Configuration

Optional, at `~/.config/tt/config.toml` (override with `TT_CONFIG`):

```toml
ot_command = "ot"
timezone = "America/New_York"
week_start = "monday"
report_format = "text"
```

| Setting | Default | Meaning |
|---|---|---|
| `ot_command` | `ot` | Path or command name for the OpenTodo CLI |
| `timezone` | system timezone | Timezone for day and week boundaries. `ot` exposes no timezone setting, so there is nothing to inherit. |
| `week_start` | `monday` | First day of a report week. `ot` exposes no week-start setting either. |
| `report_format` | `text` | Default report format: `text`, `json` or `csv` |
| `timeout_seconds` | `30` | How long to wait for an `ot` command before killing it |
| `max_entries` | `5000` | `--limit` for `ot time list`; a result that fills it is reported as incomplete |
| `version_check` | `true` | Run `ot version --json` before other calls |

Unknown keys are rejected, so a credential cannot be left here by mistake.
`--ot-command` and `--timezone` override the file for one run.

## Exit codes

1–5 mirror `ot`'s own, so a wrapper script can treat them the same way.

| Code | Meaning |
|---|---|
| 0 | Success |
| 1 | OpenTodo rejected the request or failed |
| 2 | Usage or validation error |
| 3 | Authentication or configuration problem |
| 4 | OpenTodo server unreachable — **nothing was recorded anywhere** |
| 5 | Not permitted (read-only token, read-only mode) |
| 6 | The operation has no documented OpenTodo equivalent |
| 7 | The `ot` client is missing, unusable, or did not finish |
| 8 | No timer is running |

`tt` never reports success that OpenTodo did not confirm. When `ot` returns no
usable detail — its `time start` and `time stop` JSON is undocumented — `tt`
re-reads the entry, and if that fails it says what it does not know rather than
printing a duration it computed itself.

## Development

```bash
cargo test          # 117 tests: unit plus integration
cargo build --release
```

Integration tests run the real `tt` binary against a mock `ot` executable that
answers only the commands a test installs. Anything else fails with
`unexpected_command`, so an invented `ot` subcommand breaks the suite. `TT_NOW`
pins the clock so report periods and elapsed times are deterministic.

### Testing against a real OpenTodo server

[`testing/`](testing/) has a container, sample data, and nine opt-in tests that
run `tt` against a live instance — the ones that can actually close the
unverified items in the compatibility document:

```bash
cd testing && docker compose up -d   # then create the owner account in a browser
ot login --profile tt-test http://localhost:8080
export OPENTODO_PROFILE=tt-test
./seed.sh                            # sample tasks and 3h45m of known time
./run-live-tests.sh
```

Those tests carry `#[ignore]`, so `cargo test` never touches a server, and CI
asserts they stayed that way. See [`testing/README.md`](testing/README.md).

### Deploying the server

[`deploy/`](deploy/) holds two ways to run OpenTodo itself — behind HTTPS,
with a Keycloak realm that brokers GitHub accounts into OpenTodo's OIDC
sign-in. They share one realm definition, so the two cannot drift.

| | |
|---|---|
| [`deploy/k8s/`](deploy/k8s/) | Kubernetes, as kustomize overlays: a production cluster, and a `kind` cluster you can throw away. `cd deploy/k8s/kind && ./bootstrap.sh` |
| [`deploy/quadlet/`](deploy/quadlet/) | A single host, as podman quadlets, set up with Ansible and driven by a justfile. `cd deploy/quadlet && just deploy` |

Either way, [`deploy/bin/ot-credentials.sh`](deploy/bin/ot-credentials.sh)
points your local `ot` at the result with an API token — which is the only
credential `tt` needs, and the only one that works on an account created
through single sign-on.

### Containers

[`containers/`](containers/) builds two images from one Containerfile: a
Toolbox/Distrobox with `tt` and `ot` already on `PATH`, and a test image that
runs `cargo test` and the opt-in live tests against a mounted working tree.

```bash
cd containers && ./build.sh
toolbox create --image localhost/tt-toolbox:latest tt
./run-tests.sh unit
```

It is infrastructure for the server, not for `tt`. The one thing it changes
for `tt` users: an account created through SSO has no password, so give `ot`
an API token (`ot auth set-token`, or `OPENTODO_TOKEN`) rather than
`ot login`. See [`deploy/k8s/README.md`](deploy/k8s/README.md).

### Continuous integration

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) runs on every pull
request and on pushes to `main`:

| Job | Blocking | What it does |
|---|---|---|
| `test` | yes | Builds, runs all 117 tests, verifies the live tests are still opt-in, parses the shell scripts |
| `msrv` | yes | `cargo check` on Rust 1.88, the version `rust-version` claims |
| `lint` | no, for now | `cargo fmt --check` and `cargo clippy -D warnings` |
| `release` | — | Only on `main`, only after `test` and `msrv` pass: builds the release binary, smoke-tests it, uploads it as an artifact |

`lint` is advisory because this code has not been through rustfmt or clippy
yet; drop its `continue-on-error` once it is green.

[`.github/workflows/live.yml`](.github/workflows/live.yml) is manual only
(Actions → Run workflow). It starts the real OpenTodo image and checks
`/healthz`, captures `ot schema` as an artifact for updating the compatibility
document, and — if you point the `OPENTODO_URL` and `OPENTODO_TOKEN` secrets at
a throwaway instance — runs the live tests.

```text
src/
  main.rs         wiring and exit codes
  cli.rs          command-line surface
  config.rs       settings
  clock.rs        injectable "now"
  report.rs       periods, timezone clipping, aggregation
  output.rs       text, JSON and CSV rendering
  commands/       one module per command group
  ot/             the OpenTodo adapter — the only code that knows about `ot`
    mod.rs        operations, argument construction, exit-code mapping
    model.rs      documented JSON shapes
    runner.rs     process execution, timeouts, mock runner
    redact.rs     credential scrubbing for anything `ot` prints
```

Nothing outside `src/ot/` knows that `ot` exists.
