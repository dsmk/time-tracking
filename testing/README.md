# Test OpenTodo server

A throwaway OpenTodo instance in a container, sample data, and the opt-in tests
that run `tt` against it.

Nothing here runs in ordinary CI. `cargo test` never touches a server — the
live tests are `#[ignore]`d and a CI step asserts they stayed that way. This
directory is how you close the **UNVERIFIED** items in
[`../docs/opentodo-compatibility.md`](../docs/opentodo-compatibility.md) §11.

> **One step is manual.** OpenTodo creates its owner account through the web
> interface — the first visitor from a loopback address becomes the owner.
> There is no documented way to create that account from the command line, so
> the setup below cannot be fully scripted, and the live tests cannot run
> unattended in CI against a fresh container. Everything after the account
> exists is scripted.

> **These scripts write data.** Use the throwaway container, or at minimum a
> dedicated `ot` profile for a scratch account. `seed.sh` prints the target
> server and account and asks before writing.

## What is here

| File | Purpose |
|---|---|
| `docker-compose.yml` | The test server, from the documented self-hosting variables |
| `.env.example` | Port, base URL and image tag |
| `seed.sh` | Creates sample tasks and time entries with documented `ot` commands, and writes `.live-env` |
| `reset.sh` | Removes what `seed.sh` created |
| `run-live-tests.sh` | Runs the opt-in tests in `../tests/live.rs` |
| `capture-findings.sh` | Dumps what a real `ot` prints, for updating the compatibility doc |
| `seed/sample-tasks.json` | The same tasks in OpenTodo's documented `json-generic` import format |

## Requirements

- Docker with Compose
- The OpenTodo CLI 0.9+ (`go install github.com/OpenTodoApp/opentodo/cmd/ot@latest`)
- `jq`, and GNU `date` (on macOS: `brew install coreutils` and put `gnubin`
  on `PATH`)

## 1. Start the server

```bash
cd testing
cp .env.example .env        # optional: change the port or pin another tag
docker compose up -d
```

Wait for it to come up. The image ships its own healthcheck; from the host:

```bash
curl -fsS http://localhost:8080/healthz
# {"status":"ok","version":"..."}
```

## 2. Create the owner account (manual)

Open <http://localhost:8080> in a browser. Because the request comes from
loopback, the first visitor creates the owner account — fill in a throwaway
email and password.

If you are reaching the container from another machine, the server prints a
one-time setup code at startup instead:

```bash
docker logs tt-test-opentodo | grep -i setup
```

Or set `OPENTODO_SETUP_TOKEN` (16+ characters) in `.env` before the first
start and use that.

Locked out? `docker compose exec opentodo /opentodo reset-owner-password`
prints a one-time reset link.

## 3. Point `ot` at it — on a separate profile

A profile keeps the test credentials away from any real OpenTodo account you
use day to day:

```bash
ot login --profile tt-test http://localhost:8080
export OPENTODO_PROFILE=tt-test
ot auth status
```

`tt` stores no credentials of its own; it uses whatever `ot` is configured
with, so exporting `OPENTODO_PROFILE` is all it takes.

## 4. Seed sample data

```bash
./seed.sh
```

It creates a `TT Test` project with three tasks and four time entries, in the
last complete Monday–Sunday week:

| Day | Task | Times | Duration |
|---|---|---|---|
| Mon | Design review | 09:00–10:00 | 1h 00m |
| Tue | Planning | 14:00–15:30 | 1h 30m |
| Wed | Design review | 23:00–23:45 | 45m |
| Thu | Write report | `30m` duration form, ends 17:00 | 30m |
| | | **total** | **3h 45m** (13500s) |

Why these: the Wednesday entry sits just before local midnight, so it catches a
timezone mismatch between `ot` and `tt`; the Thursday entry uses OpenTodo's
documented duration form rather than explicit times; and `Design review` gets
two entries so the per-task grouping has something to merge.

No entry crosses midnight — OpenTodo rejects an end time before the start, and
`ot time log --from/--to` are times on a single day, so a midnight-crossing
entry cannot be created through a documented interface. The split-across-days
arithmetic is covered by the unit tests in `src/report.rs` instead.

`seed.sh` refuses to run if that week already has entries, since the live tests
assert exact totals. It writes `.live-env` with the ids, dates and expected
durations.

### Bulk alternative

`seed/sample-tasks.json` is the same task list in OpenTodo's documented
`json-generic` format (a `tasks` array plus an optional `projects` array). Load
it through **Settings → Data → Import** or `ot import`. It creates tasks only —
the import format does not carry time entries, so `seed.sh` is what the live
tests need.

## 5. Run the live tests

```bash
./run-live-tests.sh
```

Nine tests in [`../tests/live.rs`](../tests/live.rs), run single-threaded
because OpenTodo allows one running timer per account:

- real `ot time list --json` parses into entries tt can report on
- report totals, daily totals and per-task totals match the seeded data exactly
- the 23:00 entry lands on the right local day
- start → status → stop round trip, read back from the server
- `tt stop` with nothing running exits 8 **without** calling `ot time stop`
- a second `tt start` is refused and leaves the first entry untouched
- an unreachable server fails and leaves no entry behind
- `tt sync status` still refuses to claim "synchronized"; `tt sync` exits 6
- a read-scoped token cannot start a timer (needs `TT_LIVE_READ_TOKEN`)

For the last one:

```bash
ot token create tt-read --scope read
export TT_LIVE_READ_TOKEN=<the printed token>
```

## 6. Record what you learned

```bash
./capture-findings.sh                 # read-only
./capture-findings.sh --with-writes   # also captures time start/stop output
```

This writes `findings/`, which answers most of
`docs/opentodo-compatibility.md` §11 — `ot schema` alone covers the real flags,
output schemas and limits, and needs no server. Update that document and delete
the items you have closed.

Check the files for credentials before pasting them anywhere;
`auth-status.json` names your account and token.

Then walk [`../docs/live-check.md`](../docs/live-check.md) for the parts that
need a human: confirming entries appear in the web interface, and the offline
behaviour.

## Cleaning up

```bash
./reset.sh                     # remove the seeded tasks and entries
docker compose down            # stop the server, keep the data
docker compose down -v         # stop it and delete the volume
```
