# OpenTodo compatibility investigation

Required by `SPEC.md` §3. This records what the `tt` adapter is allowed to rely
on, where each fact came from, and — importantly — what has **not** been
verified against a live OpenTodo instance.

- Investigation date: 2026-10-28
- Sources:
  - <https://open-todo.app/docs/cli/> (OpenTodo CLI, `ot`)
  - <https://open-todo.app/docs/time-tracking/> (time tracking user guide)
  - <https://open-todo.app/docs/api/> (HTTP API, entity fields)
  - <https://open-todo.app/docs/offline-and-sync/> (offline and sync user guide)

## 0. Evidence levels

Every claim below is tagged:

| Tag | Meaning |
|---|---|
| **DOC** | Stated in the OpenTodo documentation listed above. |
| **MOCK** | Exercised by `tt`'s automated tests against a mock `ot` executable. The *contract* is DOC; the mock only proves `tt` handles it. |
| **UNVERIFIED** | Not stated in the documentation and not tested. `tt` must not depend on it, or must degrade safely. |
| **BINARY** | Observed by running a real `ot` binary, built from upstream source. Not a live *server* — no OpenTodo instance was involved — so it settles only what the client does offline. |

**No live OpenTodo server was available during this investigation.** Nothing
below is tagged "tested against a live instance". A real `ot` binary has since
become available — [`containers/`](../containers/) builds one — and the few
facts taken from it are tagged **BINARY**.
Section 11 lists exactly what must still be checked before the acceptance
criteria in `SPEC.md` §12 can be called met.

## 1. Versions

| Item | Value | Evidence |
|---|---|---|
| Documented `ot` version | 0.13.0 | DOC |
| Minimum version `tt` accepts | 0.9.0 (the release that introduced time tracking) | DOC — `SPEC.md` targets 0.9+ |
| Output contract version | `output_version` is `1`; within a version, fields are only added | DOC |
| Version probe | `ot version --json` reports `output_version` | DOC |

`tt` runs `ot version --json` once per invocation before any other call. It
fails with an actionable error when `output_version` is present and is not `1`,
or when the reported client version parses and is below 0.9.0. The adapter
searches a small set of candidate keys (`version`, `ot_version`,
`client_version`, `cli_version`) and *skips* the version comparison rather
than failing when none is present. The probe can be turned off with
`version_check = false`.

**The key is `version`** — BINARY, 2026-10-29. An `ot` built from upstream
source at `v0.16.0` (the `ot` stage of [`containers/Containerfile`](../containers/Containerfile))
prints:

```json
{"output_version":1,"version":"v0.16.0"}
```

so `output_version` is 1 as documented, and the leading `v` is stripped by
`parse_semver`. Note that a plain `go install` leaves `main.version` at its
`"dev"` default — upstream's Makefile stamps it with
`-ldflags "-X main.version=…"`. An unstamped `ot` makes `tt` warn that it
could not determine the client version, on every command. That is a build
artefact, not a protocol difference.

## 2. Time-tracking commands (DOC)

Quoted from the CLI docs:

```text
ot time start 8c2a --note "review"
ot time stop
ot time log 8c2a 1h30 --on yesterday --note "review"
ot time log 8c2a --from 09:00 --to 09:25
ot time list --from 2026-10-12 --to 2026-10-18
ot time export --from 2026-10-12 --to 2026-10-18 -o week.csv
```

Additional documented flags on `ot time list`: `--task`, `--running`,
`--limit`.

`tt` uses `time start`, `time stop`, and `time list` only. It does **not** use
`time log` (no `tt` command creates manual entries yet) and does **not** use
`time export` (`tt` needs per-entry timestamps for boundary clipping, which the
JSON from `time list` provides and the CSV column list does not document).

### Task association (DOC)

`ot time start` takes a task identifier as a positional argument; there is no
documented way to start an entry with no task. OpenTodo's data model is
"a time entry is one stretch of work **on a task**". Consequently:

- `tt start "text"` sends `text` as `--note`, not as a task name.
- A task reference is mandatory. `tt start` resolves it from `--task`, or by
  prompting with `ot list --json` when stdin and stderr are both a terminal.

### One timer at a time (DOC)

"Only one timer runs at a time. Starting another task ends the current entry."
`tt start` therefore refuses to start when a timer is already running unless
`--switch` is passed, so the implicit stop is always explicit in `tt`.

## 3. Machine-readable output (DOC)

- "Every command accepts `--json`", and stdout then contains only JSON.
- Field names match the REST API; lists are arrays, single results are entities.
- Error envelope: `{"error":{"code":"…","message":"…","exit":N}}`, plus one
  `ot:`-prefixed line on stderr.
- `ot schema` prints every command, flag, exit code, error code and JSON Schema,
  needs no server, and exits 0 offline.

### Time-entry entity (DOC, from the API page)

```text
{id, taskId, userId, startedAt, endedAt, minutes, note, deviceId,
 taskTitle, projectId, projectName}
```

- `startedAt` / `endedAt` are **Unix milliseconds**; `endedAt` is `null` while
  the entry is running.
- `minutes` is whole minutes, counting up to server time for a running entry.
- Keys are camelCase.

The HTTP list envelope is `{entries, nextCursor?, serverTime}`. Whether the
**CLI** prints that envelope or a bare array is **UNVERIFIED** — the CLI page
says only that "lists are arrays". The adapter therefore accepts either a bare
array or an object with an `entries` array, and reads `serverTime` when present.
Snake_case aliases are accepted defensively.

The JSON emitted by `time start` and `time stop` is **explicitly undocumented**.
`tt` parses it as a time entry when it can, and otherwise re-queries
`ot time list` for the authoritative entry instead of fabricating one.

### Exit codes (DOC)

| Code | Meaning |
|---|---|
| 0 | success, including "already in that state" |
| 1 | the server rejected the request or failed |
| 2 | usage or local validation error |
| 3 | authentication or configuration error |
| 4 | server unreachable within 15 s |
| 5 | not permitted (`insufficient_scope`, `read_only`) |

`tt` mirrors 1–5 for failures that originate in `ot`, and adds 6 (capability not
supported), 7 (`ot` missing or unusable) and 8 (no active timer).

### Scopes (DOC)

`time start`, `time stop` and `time log` are writes: a `read` token exits 5
with `insufficient_scope`. `time list` and `time export` work with a `read`
token.

## 4. Running timer (DOC + gap)

There is **no** documented "show the current timer" command. The documented
options are `ot time list --running` and `ot time stop` ("exit 1 when nothing
runs"). `tt status` uses `ot time list --running --json`, which is a read and
therefore safe to run repeatedly.

`tt stop` checks `--running` first and exits 8 without calling `ot time stop`
when nothing is running, so "no active timer" never reaches OpenTodo as a failed
write, and `ot`'s exit 1 (used both for "nothing runs" and for real server
failures) is never ambiguous to the user.

## 5. Offline behavior (DOC — and the distinction that matters)

The two user-facing docs say different things about different clients, and
conflating them would be the single easiest way to ship a false claim:

- **Apps** (web/desktop/mobile): "OpenTodo keeps a full copy of your tasks on
  each device", time entries "are stored on the device first, so they work
  offline and sync with your tasks", and "changes wait on the device and are
  sent as soon as there's a connection again".
- **The `ot` CLI**: "The CLI is online-only." It "keeps no local copy of your
  tasks" and makes no changes when the server is unreachable. Every write goes
  through the REST API.

`tt` is built on `ot`, so **`tt` has no offline support at all** — neither reads
nor writes. When the server is unreachable, `ot` exits 4 and `tt` reports that
plainly and exits 4. `tt` never queues, caches, or writes an entry anywhere
else, which also satisfies `SPEC.md` §7's "no parallel source of truth".

The offline acceptance tests in `SPEC.md` §11 are therefore **not applicable to
`tt` as built**, and no offline capability is claimed. They would only become
relevant if `tt` were re-based onto a client that is documented as
offline-capable.

## 6. Sync status (DOC — exposed only in the app)

The sync states OpenTodo defines are app UI states: *Synced*, *Syncing…*,
*N pending*, *Offline · N pending*, *Sync error*, *Signed out*, *Locked*.

**No documented `ot` command or REST route reports device sync state**, and
`ot` has no local change queue to report on. So the honest mapping for
`SPEC.md` §6.7 is:

| `SPEC.md` state | What `tt sync status` can say |
|---|---|
| Synchronized | **Never asserted.** `ot` writes straight to the server; there is no device state to call "synced". |
| Sync in progress | Not observable. |
| Offline or unreachable | Reported — `ot` exits 4. |
| Pending local changes | Reported as *none possible*: `ot` keeps no local queue (DOC). |
| Conflict or failure | Reported when a command returns `conflict`/`idempotency_conflict`, otherwise not observable. |
| Sync status unavailable | **The default**, and what `tt sync status` reports. |

`tt sync status` probes reachability with `ot auth status --json` (a read) and
reports reachability and authentication *separately* from sync state, so a
successful command is never dressed up as "synchronized".

`tt sync` has no documented operation to call: the only `sync` verbs in the CLI
are `ot issues sync` (issue import) and `ot vault sync` (the Markdown vault),
neither of which syncs time entries. `tt sync` prints guidance and exits 6
rather than inventing a command.

## 7. Conflicts and idempotency (DOC)

- Writes converge under "per-field last-writer-wins".
- `--idempotency-key` is documented for `add` and `project add` **only**; it is
  not documented for `ot time start`. `tt` therefore does not pass it, and a
  retried `tt start` may create a second entry. This is called out in the README.
- Documented error codes include `conflict`, `idempotency_conflict`,
  `invalid_input`, `unauthorized`, `forbidden`, `insufficient_scope`,
  `not_found`, `rate_limited`.

## 8. Authentication and instance configuration (DOC)

Handled entirely by `ot`: `ot login`, `ot auth set-token`, `ot auth status`,
`ot auth logout`; config at `~/.config/opentodo/config.json` (mode 0600);
profiles via `--profile` / `OPENTODO_PROFILE`; `OPENTODO_URL`, `OPENTODO_TOKEN`,
`OPENTODO_READ_ONLY`.

`tt` stores **no credentials and no server URL**. Its config file rejects
unknown keys, which makes an accidental `token = "…"` a hard error. Error text
from `ot` is passed through a redactor before display.

## 9. Historical queries and reporting limits

| Question | Answer | Evidence |
|---|---|---|
| Can entries be queried by date range? | Yes — `ot time list --from --to` | DOC |
| Are `--from`/`--to` local days? | The API's `from`/`to` are local days `YYYY-MM-DD` | DOC |
| Can the timezone be chosen? | The **API** takes `tz` (IANA). The **CLI** has no documented `--tz` flag. | DOC / gap |
| Is there a documented default or maximum `--limit`? | No | UNVERIFIED |
| Is there CLI pagination? | The API has `cursor`/`nextCursor`; the CLI documents only `--limit` | DOC / gap |
| Is there a built-in weekly report? | No. Statistics (by day, by project) is a web-app feature. | DOC |

Consequences, all implemented in `tt`:

1. **No CLI `--tz`.** `tt` does not guess which zone `ot` applies to
   `--from`/`--to`. It widens the requested window by one day on each side, then
   clips every entry itself in the configured timezone using the absolute
   `startedAt`/`endedAt` millisecond timestamps. Day and week boundaries in a
   `tt` report are `tt`'s, computed in `timezone`, and are reported in the
   output.
2. **No server-side weekly report.** `tt report` is derived from `time list`
   data, per `SPEC.md` §5 option 2.
3. **No documented pagination.** `tt` always passes an explicit `--limit`
   (`max_entries`, default 5000) and, when the number of entries returned equals
   that limit, marks the result incomplete and warns, instead of silently
   reporting a short week.
4. **No week-start setting is exposed by `ot`**, so `tt` uses its own
   `week_start` (default `monday`) and prints it with every report.
5. **Durations are computed from timestamps, not from `minutes`**, because a
   report has to clip entries at period boundaries and `minutes` is already
   rounded.

## 10. What the adapter is allowed to invoke

Exactly these, and nothing else:

```text
ot version --json
ot auth status --json
ot list --json                      # task picker only
ot time list --json [--from D] [--to D] [--task ID] [--running] [--limit N]
ot time start <TASK> [--note TEXT] --json
ot time stop --json
```

Every one appears verbatim in the CLI documentation. The integration tests use a
mock `ot` that fails with an "unexpected command" error on anything else, so an
invented subcommand breaks the test suite (MOCK).

Process handling (all MOCK-tested): argument vectors only — never a shell
string; stdout, stderr and exit status captured separately; a configurable
timeout (`timeout_seconds`, default 30 s, above `ot`'s own 15 s server timeout)
after which the child is killed; termination by signal reported as such;
`ot` missing reported as exit 7 with installation guidance.

## 11. Still to verify against a live instance

Before `SPEC.md` §12 can be signed off, stand up a test instance
(`testing/README.md`), run `testing/capture-findings.sh` and the opt-in tests in
the live tests, walk the manual steps in `docs/live-check.md`, and update this
file with the results:

1. The real shape of `ot time list --json` — bare array vs `{entries,…}`
   envelope, and whether `serverTime` is present.
2. The real shape of `ot time start --json` and `ot time stop --json`.
3. ~~Which key in `ot version --json` holds the client version.~~ **Answered**
   (see §1): it is `version`. `containers/` builds an `ot` that can be asked
   without a server.
4. The default and maximum values of `ot time list --limit`, and what happens
   when more entries exist than the limit.
5. Which timezone `ot` uses to interpret `--from`/`--to` (confirming that the
   one-day widening in §9 is sufficient).
6. That an entry started by `tt start` appears in the OpenTodo web interface
   with the same start time, task and note.
7. That `ot time stop` really exits 1 (and with which error `code`) when no
   timer runs.
8. The shape of `ot list --json` task entities used by the picker.

Run `ot schema > schema.json` on a real install first: per the docs it answers
items 1–4 and 8 offline, without touching a server. Since
[`containers/`](../containers/) now builds a real `ot`, that no longer needs
an install at all:

```bash
cd containers && ./build.sh test
podman run --rm --network=none tt-test:latest shell ot schema > schema.json
```
`testing/capture-findings.sh` does that and the rest in one go, and the manual
`Live OpenTodo checks` workflow captures the schema as a CI artifact.

Items 1, 2, 5, 6 and 7 are also asserted by
`internal/integration/live_test.go`, so once a seeded
instance exists they are checked rather than eyeballed.
