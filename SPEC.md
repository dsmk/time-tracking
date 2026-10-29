# OpenTodo Time Tracking CLI — Specification

## 1. Purpose

Build a Rust or Go command-line client that makes OpenTodo time tracking fast
and convenient from a terminal.

OpenTodo is the system of record for tasks and time entries. The CLI uses
OpenTodo's documented command-line interface or another officially documented
integration interface. OpenTodo's web interface is used to browse and manage
the same data, and OpenTodo is responsible for synchronizing data between
supported clients.

The CLI may add convenience features such as shell-friendly commands, weekly
reports, and exports. It must not create a separate authoritative time-tracking
database or write directly to OpenTodo's internal files or database.

References:

- OpenTodo CLI: https://open-todo.app/docs/cli/
- OpenTodo time tracking: https://open-todo.app/docs/time-tracking/

## 2. Design principles

1. **OpenTodo owns the data.** Tasks and time entries are stored and synchronized
   by OpenTodo using its supported mechanisms.
2. **Use documented interfaces.** Do not depend on undocumented APIs, database
   schemas, or internal application files.
3. **Keep the CLI thin.** Reuse OpenTodo's time-entry behavior instead of
   implementing a second timer or data model.
4. **Keep reports honest.** If OpenTodo does not provide a required report,
   calculate it from data obtained through a documented interface and describe
   any limitations.
5. **Offline behavior must be verified.** Do not claim that time entries can be
   created offline or automatically synchronized until tested against the
   documented OpenTodo behavior.
6. **The OpenTodo web interface remains available.** Users must be able to view
   and manage the time-tracking data using OpenTodo's supported web interface.

## 3. Required compatibility investigation

Before implementing the adapter, inspect the current OpenTodo CLI and time
tracking documentation and record findings in
`docs/opentodo-compatibility.md`.

Document:

- Minimum supported OpenTodo version; target 0.9 or later.
- The documented `ot` commands for starting, stopping, listing, and editing
  time entries, if provided.
- How those commands identify or associate a time entry with a task.
- Whether commands return machine-readable output such as JSON.
- Whether the CLI can operate while offline.
- Whether changes made offline are synchronized automatically after reconnect.
- How sync status, failures, and conflicts are reported.
- Whether operations support stable IDs or idempotency.
- Authentication and instance configuration.
- Any limits on querying historical entries for reports.
- Which behavior was tested rather than inferred from documentation.

Do not invent `ot` command syntax. The adapter must be based on commands or APIs
confirmed in the documentation.

## 4. Architecture

```text
┌──────────────────┐
│ User             │
└────────┬─────────┘
         │
         ├─────────────────────┐
         ▼                     ▼
┌──────────────────┐  ┌──────────────────────┐
│ `tt` CLI         │  │ OpenTodo web UI      │
│ Rust or Go       │  │ tasks and time data  │
└────────┬─────────┘  └──────────┬───────────┘
         │                       │
         │ documented interface  │
         └──────────┬────────────┘
                    ▼
           ┌───────────────────┐
           │ OpenTodo          │
           │ data and sync     │
           └───────────────────┘
```

The CLI should use the documented `ot` command-line client where suitable.
If the CLI documentation provides a supported API or SDK that is more
appropriate, that may be used instead.

The implementation must not call the `ot` command by constructing a shell
string. Invoke it with an argument array through the process API, and safely
handle user-provided descriptions, task names, and identifiers.

If the `ot` client is not installed or cannot connect to a configured OpenTodo
instance, print an actionable error. Do not silently fall back to writing
records somewhere else.

## 5. Scope

### Included

- A Rust or Go CLI named `tt` (working name).
- Integration with documented OpenTodo time-tracking operations.
- Convenient start, stop, status, and listing commands.
- Weekly and date-range reports, if supported by OpenTodo or safely derivable
  from documented output.
- Optional JSON and CSV report output.
- Sync-status visibility when OpenTodo exposes it.
- Automated tests using a mock OpenTodo command or provider.

### Not included

- A replacement web interface.
- A replacement sync service or account system.
- Direct access to OpenTodo's internal database or files.
- A second authoritative SQLite database for time entries.
- Claims of offline support not demonstrated by the compatibility investigation.
- Direct manipulation of OpenTodo's private network protocols.

## 6. CLI behavior

The commands below specify the desired user experience. The implementation must
map them to the actual, documented OpenTodo CLI/API operations. These are
`tt` commands, not claims about OpenTodo's command syntax.

### 6.1 `tt start`

```text
tt start "Design review"
tt start "Design review" --task TASK_ID
```

Behavior:

- Start a time-tracking entry using OpenTodo's supported operation.
- If `--task` is supplied, associate the entry with that OpenTodo task using a
  supported task identifier or lookup.
- If task selection is interactive, allow the user to choose a task when
  running in a terminal.
- In non-interactive mode, require sufficient task information.
- Display the result returned by OpenTodo, including any available entry or task
  identifier and start time.
- Do not start a second timer unless OpenTodo supports that behavior and the
  user explicitly requests it.

### 6.2 `tt stop`

```text
tt stop
```

Behavior:

- Stop the active time-tracking entry using OpenTodo's supported operation.
- If no timer is active, report that clearly and exit without changing data.
- Display the completed entry and duration when OpenTodo returns that
  information.

### 6.3 `tt status`

```text
tt status
tt status --json
```

Show the currently running timer, if available through the documented
interface. If none is running, print a concise message.

JSON output is supported only if the OpenTodo CLI/API provides structured data
or the adapter can reliably construct JSON from documented output. Do not parse
unstable, human-oriented output as though it were a stable API.

### 6.4 `tt log`

```text
tt log
tt log --today
tt log --from 2026-10-20 --to 2026-10-27
tt log --task TASK_ID
```

List time entries using supported OpenTodo queries or exports.

Options:

- `--today`: entries touching today in the selected timezone
- `--from DATE`: inclusive start date
- `--to DATE`: inclusive end date
- `--task ID`: filter by task
- `--format text|json`: output format, if the integration can provide reliable
  structured data

If OpenTodo cannot query or export the needed entry data through a documented
interface, report that limitation rather than scraping its UI or private
storage.

### 6.5 `tt report week`

```text
tt report week
tt report week --week 2026-10-27
tt report week --format json
tt report week --format csv
```

Generate a weekly report using either:

1. OpenTodo's documented reporting capability, if it provides the required
   period and grouping; or
2. Time-entry data returned by a documented CLI/API operation.

A report should include, where the available data permits:

- Report date range
- Total tracked duration
- Daily totals
- Duration by task
- Unassigned or taskless duration, if applicable
- An indication if entries are still running or data is incomplete

The week start and timezone behavior must be documented. Do not assume OpenTodo's
week-start or timezone settings; either honor the relevant OpenTodo setting or
provide a clearly documented CLI option.

### 6.6 `tt report range`

```text
tt report range --from 2026-10-01 --to 2026-10-31
```

Dates are inclusive local calendar dates. The report must use the configured or
OpenTodo-provided timezone consistently and clip entries at the report
boundaries.

If required timezone or historical data is unavailable through the supported
interface, explain the limitation.

### 6.7 `tt sync status`

```text
tt sync status
```

Show OpenTodo synchronization status if exposed by the supported CLI/API. The
command must distinguish among:

- Synchronized
- Sync in progress
- Offline or unreachable
- Pending local changes
- Conflict or failure
- Sync status unavailable

Do not infer “synchronized” merely because a CLI command succeeded.

### 6.8 `tt sync`

```text
tt sync
```

Invoke an OpenTodo synchronization operation only if the documented CLI/API
provides one. If OpenTodo synchronizes automatically and has no explicit sync
command, display the appropriate guidance instead of inventing one.

## 7. Offline behavior

Offline behavior is a capability to verify, not an assumption.

### Read and report behavior

- If OpenTodo's supported CLI provides access to locally cached entries while
  offline, `tt log` and reports may use that data.
- Clearly indicate that the report is based on local data and may not include
  changes from other devices.
- If offline reads are unavailable, report that the configured OpenTodo client
  is unreachable.

### Start and stop behavior

- Permit offline start or stop only if the documented OpenTodo client supports
  that operation offline and synchronizes it safely later.
- Test that a completed offline action appears in OpenTodo after reconnection.
- Verify how duplicate submissions, multiple offline timers, and conflicting
  changes are handled.
- If offline writes are not supported, fail clearly; do not create an unrelated
  local entry that appears synchronized.

### No parallel source of truth

The MVP must not create an independent local time-entry database to simulate
offline tracking. A local cache or temporary state may be added later only if
the design specifies how it reconciles with OpenTodo and prevents duplicate or
lost entries.

## 8. OpenTodo adapter

Keep OpenTodo-specific process, API, and output handling behind an adapter.

Suggested conceptual operations:

```text
start_timer(task_reference, description)
stop_timer()
get_running_timer()
list_time_entries(filters)
get_sync_status()
sync()
```

Only implement operations that correspond to documented and verified
OpenTodo capabilities. An unsupported operation must return a clear
“not supported by this OpenTodo version/client” error.

The adapter should:

- Check that the required `ot` client is installed.
- Detect or validate the configured OpenTodo instance.
- Check the OpenTodo version when the CLI provides that information.
- Capture standard output, standard error, and exit status separately.
- Avoid exposing credentials in logs or error output.
- Handle timeouts and interrupted commands.
- Preserve OpenTodo's original error details where safe.
- Prefer documented structured output over parsing terminal formatting.
- Have unit tests using a mock process or mock provider.

## 9. Configuration

Configuration is for `tt` behavior and the OpenTodo client location. Do not
duplicate OpenTodo credentials or sync configuration if OpenTodo already
manages them.

Example:

```toml
ot_command = "ot"
timezone = "America/New_York"
week_start = "monday"
report_format = "text"
```

Settings:

| Setting | Default | Meaning |
|---|---|---|
| `ot_command` | `ot` | Path or command name for the OpenTodo CLI |
| `timezone` | OpenTodo setting or system timezone | Timezone used for reports, if supported |
| `week_start` | OpenTodo setting or `monday` | First day of report week |
| `report_format` | `text` | Default report format |

Credentials should remain under OpenTodo's supported credential management.
Do not write secrets to the `tt` configuration file.

## 10. Error handling

The CLI must provide actionable errors for:

- `ot` is missing or cannot be executed
- Unsupported OpenTodo version or operation
- OpenTodo instance is unreachable
- Authentication failure
- No active timer
- Invalid task identifier
- Invalid date range or timestamp
- Missing historical data required for a report
- Sync failure or unresolved conflict

The CLI must not claim that an operation succeeded unless the OpenTodo
operation returned success and the result was interpreted reliably.

## 11. Testing

### Unit tests

Test:

- Command and argument construction
- Safe handling of descriptions and task IDs
- Parsing documented structured output
- Report calculations and date-boundary clipping
- Timezone and week-boundary behavior
- Error and exit-status handling
- Unsupported capability behavior

### Integration tests

Use a mock `ot` executable or test provider to verify:

- Start, stop, status, and list workflows
- OpenTodo errors are surfaced accurately
- JSON/CSV output is valid
- Missing `ot` client is handled cleanly
- Unsupported commands are not invoked
- Sync status is not falsely reported as success

If an OpenTodo test instance is available, add opt-in integration tests that
exercise the documented commands. Do not make ordinary automated tests depend
on a live service or real user account.

### Offline acceptance tests

Before claiming offline support, test:

1. Disconnect the client from the network.
2. Start or stop a timer using the documented offline-capable client behavior.
3. Restart the CLI or OpenTodo client, if relevant.
4. Reconnect.
5. Confirm the entry appears once in the OpenTodo web interface and on another
   synchronized client.
6. Confirm conflicts and failures are visible rather than silently discarded.

## 12. Acceptance criteria

The MVP is complete when:

1. `tt` uses only documented OpenTodo interfaces.
2. Starting and stopping timers changes the same time-tracking data visible in
   OpenTodo's web interface.
3. The compatibility document records tested OpenTodo version and capabilities.
4. `tt` provides a useful status view for the running timer.
5. Weekly reports work using a documented OpenTodo query/export/reporting
   capability, or the CLI clearly reports the limitation.
6. Sync status is reported accurately when OpenTodo exposes it.
7. No separate authoritative time-entry database is created.
8. Offline behavior is either tested and supported, or explicitly reported as
   unavailable.
9. Failures do not result in misleading success messages or lost data.

## 13. Implementation sequence

1. Read the OpenTodo CLI and time-tracking documentation.
2. Create `docs/opentodo-compatibility.md` with confirmed commands, output
   formats, offline behavior, and reporting capabilities.
3. Install or configure the supported OpenTodo CLI and manually test the
   documented time-tracking workflow.
4. Build a small adapter that performs only start, stop, and status.
5. Add listing and weekly reports based on documented data access.
6. Add JSON/CSV output where structured data is available.
7. Test synchronization through the OpenTodo web interface and another
   supported client.
8. Add offline behavior only after it passes the acceptance tests above.
