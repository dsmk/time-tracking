# Time Tracker CLI — Local MVP Specification

## 1. Purpose

Build a command-line time tracker inspired by the speed and text-oriented
workflow of klog. The MVP runs entirely on one machine:

- Users can start and stop timers, create and edit entries, and generate reports.
- All data is stored locally in SQLite.
- Core tracking and reporting work without network access.
- Each entry has a stable ID and revision field so synchronization can be added
  later without redesigning the data model.

The implementation may use Rust. This specification does not require a
particular CLI framework or database library.

## 2. Product principles

1. **Local-first:** A network connection is never needed to track time.
2. **Fast common operations:** Starting, stopping, and pausing a timer should require
   only one command.
3. **No silent data loss:** Destructive operations require confirmation or an
   explicit `--yes` option.
4. **Predictable time handling:** Store absolute timestamps; use the configured
   timezone for display and reporting.
5. **Human-readable output:** Default output is suitable for a terminal.
6. **Automation-friendly output:** Commands that list or report data support
   JSON; reports also support CSV.
7. **Sync-ready records:** Use UUIDs, revisions, and deletion tombstones.

## 3. Name and command conventions

Use `tt` as the example executable name. The actual binary name may be
configured by the project.

Examples:

```text
tt start "Design review" --project alpha
tt stop
tt log
tt report week
```

Descriptions are positional text. Project and tag metadata are supplied using
flags.

## 4. Scope

### Included in the MVP

- Local SQLite database
- Start, stop, and inspect the active timer
- Add, list, edit, and delete time entries
- Project and tag metadata (using #tagname )
- Weekly and date-range reports
- Text, JSON, and CSV output where specified
- User configuration for timezone, week start, and display behavior
- Database initialization and schema migrations

### Not included in the MVP

- Server, accounts, authentication, or synchronization
- Background daemon or scheduled timer
- GUI or web interface
- Automatic activity detection
- Billing, invoicing, or payroll features
- Shared projects or multi-user data
- Natural-language date parsing beyond the formats specified below
- Import from or export to another time-tracking application

## 5. Storage and configuration locations

Use operating-system-standard directories where available.

Suggested defaults:

- Database: `$XDG_DATA_HOME/tt/tt.db`, or
  `~/.local/share/tt/tt.db` when `XDG_DATA_HOME` is unset
- Configuration: `$XDG_CONFIG_HOME/tt/config.toml`, or
  `~/.config/tt/config.toml`
- Logs, if any: `$XDG_STATE_HOME/tt/`, or `~/.local/state/tt/`

On macOS and Windows, use their standard application-data directories instead
of requiring XDG variables.

The CLI must allow overriding the database path:

```text
tt --db /path/to/tt.db ...
```

A command-line database path takes precedence over an environment variable and
the default path. If an environment variable such as `TT_DB` is implemented,
document its precedence.

The database should be created automatically on the first command that needs
it. A `tt init` command may also be provided to initialize it explicitly.

## 6. Configuration

Configuration is TOML. Every setting has a usable default.

Example:

```toml
timezone = "America/New_York"
week_start = "monday"
duration_precision = "minute"
default_project = ""
```

### Settings

| Setting | Default | Meaning |
|---|---|---|
| `timezone` | System local timezone | IANA timezone used for display and reports |
| `week_start` | `monday` | First day of the reporting week; may be any weekday |
| `duration_precision` | `minute` | Display durations to the nearest minute or second |
| `default_project` | Empty | Optional project applied to new entries |

Invalid configuration must produce an actionable error identifying the setting
and file. Unknown keys may be ignored in the MVP, but should produce a warning
if practical.

Provide:

```text
tt config show
tt config set timezone America/New_York
tt config set week_start monday
```

If `config set` is not implemented, document that settings must be edited in
the TOML file.

## 7. Data model

Use SQLite with schema migrations. Store timestamps as UTC RFC 3339 strings or
integer Unix timestamps at a documented precision. Do not mix formats.

### `entries` table

| Field | Type | Rules |
|---|---|---|
| `id` | UUID text | Primary key; generated on creation |
| `start_at` | UTC timestamp | Required |
| `end_at` | UTC timestamp | Null means timer is running |
| `description` | Text | Required; trimmed; must not be empty |
| `project` | Text | Optional; normalized by trimming whitespace |
| `created_at` | UTC timestamp | Required |
| `updated_at` | UTC timestamp | Required; updated on every mutation |
| `revision` | Integer | Starts at 1; increments on every mutation |
| `deleted_at` | UTC timestamp | Null unless soft-deleted |
| `timezone` | IANA timezone name | Timezone in effect when the entry was created |

### `entry_tags` table

| Field | Type | Rules |
|---|---|---|
| `entry_id` | UUID text | Foreign key to `entries.id` |
| `tag` | Text | Non-empty, trimmed |
| Primary key | `(entry_id, tag)` | A tag may appear only once per entry |

Tag comparisons should be case-insensitive for duplicate detection. Preserve the
first spelling entered for display.

### Data constraints

- At most one **non-deleted running entry** may exist at a time.
- By default, completed entries may not overlap another completed or running
  entry. The CLI must detect overlap before creating, editing, or starting an
  entry.
- An overlap may be explicitly allowed with `--allow-overlap`. If allowed, it
  must remain visible in reports or an overlap-warning command.
- `end_at` must be later than `start_at`.
- Deleting an entry sets `deleted_at`; it does not physically remove the row.
- Normal list and report commands exclude deleted entries.
- UUIDs are generated locally and never depend on a server.

## 8. Time and timezone behavior

- Store `start_at` and `end_at` as absolute UTC instants.
- Convert timestamps to the configured timezone when displaying entries and
  calculating calendar-day or weekly reports.
- Record the IANA timezone used at entry creation in the entry’s `timezone`
  field.
- A timestamp with `Z` or an explicit UTC offset is unambiguous and may be
  accepted directly.
- A timestamp without an offset is interpreted in the configured timezone.
- If a local time is ambiguous because of a daylight-saving transition, reject
  it and ask the user to provide an explicit offset.
- If a local time does not exist because of a daylight-saving transition,
  reject it and explain why.
- The MVP accepts RFC 3339 timestamps, for example:
  `2026-10-27T09:30`, `2026-10-27T09:30-04:00`, or
  `2026-10-27T13:30:00Z`.
- “Today” and the default report period are determined using the configured
  timezone.
- Duration is calculated from the stored instants. Do not alter stored times
  when displaying rounded durations.

## 9. CLI commands

All commands should provide `--help`. Invalid input should result in a
non-zero exit status and an actionable error message.

### 9.1 `tt init`

Initialize the database and create the required tables.

```text
tt init
tt init --db /path/to/tt.db
```

Running `init` against an already initialized database must be safe.

### 9.2 `tt start`

Start a timer at the current time.

```text
tt start "Implement login"
tt start "Implement login" --project app
tt start "Investigate alert" --project platform --tag incident --tag oncall
```

Options:

| Option | Meaning |
|---|---|
| `--project TEXT` | Assign a project |
| `--tag TEXT` | Add a tag; repeatable |
| `--at TIME` | Start at a specified timestamp; default is now |
| `--allow-overlap` | Allow overlap with another entry |

Behavior:

- Require a non-empty description.
- If `--project` is absent, use `default_project` if configured.
- If another timer is running, fail without changing it. Include the running
  entry’s description and ID in the error.
- If the proposed start time overlaps an entry, fail unless
  `--allow-overlap` is supplied.
- A timer started with `--at` is still running until stopped.
- On success, print the entry ID, description, start time, project, and tags.

### 9.3 `tt stop`

Stop the running timer at the current time.

```text
tt stop
tt stop --at 2026-10-27T12:15
```

Options:

| Option | Meaning |
|---|---|
| `--at TIME` | Stop at a specified timestamp; default is now |

Behavior:

- If no timer is running, fail with a clear message.
- Require the stop time to be later than the start time.
- Before stopping, detect overlap with other entries. On overlap, fail and leave
  the timer running unless an overlap-allowing option is provided.
- On success, print the completed entry and its duration.

### 9.4 `tt status`

Show the running timer, if any.

```text
tt status
tt status --json
```

If no timer is running, print a concise “No timer running” message and exit
successfully. For JSON output, return a documented empty representation, such as
`null`.

### 9.5 `tt add`

Create a completed entry.

```text
tt add "Code review" --start 2026-10-27T09:00 --end 2026-10-27T10:30
tt add "Code review" --start 2026-10-27T09:00 --duration 90m --project alpha
```

Required timing options:

- `--start TIME`
- Exactly one of:
  - `--end TIME`
  - `--duration DURATION`

Supported duration syntax in the MVP: integer values with `s`, `m`, or `h`
units, including combinations such as `1h30m`. For example: `90m`, `1h30m`,
`45s`.

Additional options:

| Option | Meaning |
|---|---|
| `--project TEXT` | Assign a project |
| `--tag TEXT` | Add a tag; repeatable |
| `--allow-overlap` | Allow overlap with another entry |

Behavior:

- Validate that the resulting end time is later than the start time.
- Apply `default_project` when `--project` is absent.
- Check for overlaps unless explicitly allowed.
- Print the created entry and its duration.

### 9.6 `tt log`

List entries, newest first by default.

```text
tt log
tt log --today
tt log --from 2026-10-20 --to 2026-10-27
tt log --project alpha
tt log --tag incident
tt log --limit 50
tt log --json
```

Options:

| Option | Meaning |
|---|---|
| `--today` | Show entries touching today in the configured timezone |
| `--from DATE` | Inclusive start date |
| `--to DATE` | Inclusive end date |
| `--project TEXT` | Filter by project |
| `--tag TEXT` | Filter by tag |
| `--search TEXT` | Case-insensitive substring search in description |
| `--limit N` | Limit the number of results |
| `--json` | Emit JSON |

Rules:

- `--today` cannot be combined with `--from` or `--to`.
- `--from` and `--to` are calendar dates in the configured timezone.
- If only `--from` is supplied, include entries from that date onward.
- If only `--to` is supplied, include entries through that date.
- Entries that cross a date boundary are included if any portion overlaps the
  requested period.
- Running entries appear with an “in progress” marker and a duration calculated
  through the current time.
- Normal output shows date, start, end or running status, duration,
  description, project, tags, and a short ID.
- JSON must include the full UUID and machine-readable timestamps.

### 9.7 `tt show`

Show one entry by UUID or unambiguous UUID prefix.

```text
tt show <id>
tt show <id> --json
```

If a prefix matches more than one entry, require a longer prefix. Deleted
entries are not shown unless an explicit `--include-deleted` option is provided.

### 9.8 `tt edit`

Edit an entry. At least one editable field must be supplied.

```text
tt edit <id> --description "Review revised design"
tt edit <id> --project alpha
tt edit <id> --start 2026-10-27T09:15 --end 2026-10-27T10:45
tt edit <id> --tag incident --add-tag urgent
tt edit <id> --remove-tag old-tag
```

Options:

| Option | Meaning |
|---|---|
| `--description TEXT` | Replace description |
| `--project TEXT` | Set project; use `--clear-project` to remove |
| `--start TIME` | Replace start time |
| `--end TIME` | Replace end time; only valid for a completed entry |
| `--tag TEXT` | Replace all tags; repeatable |
| `--add-tag TEXT` | Add a tag; repeatable |
| `--remove-tag TEXT` | Remove a tag; repeatable |
| `--clear-project` | Remove the project |
| `--allow-overlap` | Allow the edited entry to overlap another entry |

Editing must validate the resulting entry as a whole. Update `updated_at` and
increment `revision` on success. Failed edits must leave the entry unchanged.

### 9.9 `tt delete`

Soft-delete an entry.

```text
tt delete <id>
tt delete <id> --yes
```

Without `--yes`, ask for confirmation when attached to an interactive
terminal. If standard input is not interactive, require `--yes`.

Soft-deleting a running timer is allowed only after confirmation or `--yes`;
the CLI must clearly state that the timer is being discarded rather than
stopped. Update `deleted_at`, `updated_at`, and `revision`.

### 9.10 `tt report`

Generate time summaries.

```text
tt report week
tt report week --week 2026-10-27
tt report week --project alpha
tt report range --from 2026-10-01 --to 2026-10-31
tt report week --format json
tt report week --format csv
```

#### Weekly report

- `tt report week` reports the current week in the configured timezone.
- `--week DATE` selects the week containing that date.
- Week boundaries use `week_start` from configuration.
- The report period begins at local midnight on the first day and ends at local
  midnight on the day after the last day.
- The period is treated as a half-open interval: `[start, end)`.
- Entries crossing the period boundary contribute only the portion that falls
  within the report period.
- A running entry contributes time through the current instant, clipped to the
  report period.

#### Range report

- `--from DATE` and `--to DATE` are inclusive local calendar dates.
- Internally, calculate over the half-open interval from midnight at the start
  of `--from` through midnight after `--to`.
- Reject invalid dates and `--from` later than `--to`.

#### Report contents

Text output includes:

- Report title and date range
- Total duration
- Duration by project, including an “Unassigned” group
- Daily totals
- Duration by tag when tags are present
- An overlap warning if overlapping entries contribute to the report

Entries with no project are grouped under “Unassigned.” Totals are calculated
from actual elapsed time. A day affected by a daylight-saving transition may
therefore contain 23 or 25 elapsed hours.

Formats:

- `--format text` (default): readable terminal report
- `--format json`: structured report with ISO timestamps and durations in
  seconds
- `--format csv`: rows suitable for spreadsheet import, including period,
  group type, group name, and duration in seconds

Reports should not round values before aggregation. Apply configured display
rounding only when rendering text output.

### 9.11 `tt completion` (optional)

If supported by the CLI framework, provide shell completion for common
subcommands and options. This is not required for the MVP.

## 10. Output requirements

### Human-readable output

- Use a consistent local date/time format and document it.
- Clearly distinguish running entries from completed entries.
- Include short IDs in list output and full IDs in `show` and JSON output.
- Send diagnostics and errors to standard error.
- Send command results to standard output.

### JSON output

- Use stable field names and valid JSON.
- Do not print progress messages or decorative text to standard output when
  JSON is requested.
- Timestamps must include an explicit timezone or use UTC with `Z`.
- Durations in JSON are integer seconds.
- JSON array order must be deterministic.

### Exit status

- Exit code `0` means success.
- Non-zero means the command failed.
- Validation, database, configuration, and not-found errors should be
  distinguishable in the message. Numeric exit codes may be standardized later.

## 11. SQLite and implementation requirements

- Use parameterized SQL.
- Enable foreign-key enforcement.
- Use transactions for operations that modify an entry and its tags.
- Apply schema migrations in a transaction where possible.
- Store no essential state only in memory; a running timer must survive process
  exit and machine restart.
- Do not hold a long-running database lock while a timer is active.
- If two CLI processes attempt to start a timer concurrently, database
  constraints or a transaction must prevent two active timers from being
  created.
- Provide a useful message if the database file cannot be created or opened.
- Do not delete or overwrite an existing database during initialization.

## 12. Validation and edge cases

The implementation must handle:

- Empty or whitespace-only descriptions
- Invalid UUIDs and ambiguous ID prefixes
- Invalid dates, timestamps, durations, tags, and project names
- End times before or equal to start times
- Attempting to stop when no timer is running
- Attempting to start when a timer is already running
- Overlap detection across date boundaries
- Running entries in lists and reports
- Edits that would create an invalid duration or overlap
- Missing or invalid timezone configuration
- Daylight-saving ambiguous and nonexistent local times
- Deleted entries being excluded from normal queries and reports
- Concurrent CLI invocations

## 13. Testing requirements

### Unit tests

Test at least:

- Duration parsing and formatting
- Timestamp parsing and timezone conversion
- Week boundary calculation for each possible week-start day
- Date-range clipping for entries that cross boundaries
- Overlap detection
- Tag normalization and duplicate detection
- Report aggregation and output serialization

### Integration tests

Use a temporary SQLite database to test:

- Database initialization and repeated initialization
- Start/status/stop lifecycle
- Starting a second timer is rejected
- Manual entry creation
- Entry listing and filters
- Editing entries and tags
- Soft deletion
- Weekly and range reports
- JSON and CSV output
- A failed command leaves the database unchanged
- Two concurrent attempts to start a timer cannot create two active timers

Tests that depend on current time should use an injectable clock or another
deterministic test mechanism.

## 14. MVP acceptance criteria

The MVP is complete when a user can:

1. Initialize the local database without manual SQL.
2. Start a timer, close the CLI, reopen it, and still see the running timer.
3. Stop the timer and see a completed duration.
4. Add a completed entry with explicit start and end times or a duration.
5. List and filter entries by date, project, tag, and description.
6. Edit and soft-delete entries.
7. Generate a weekly report that respects the configured timezone and week
   start.
8. Generate JSON output for entries and reports and CSV output for reports.
9. Track time with no network connection.
10. Recover from invalid input without corrupting or partially changing data.

## 15. Suggested project layout

This is a suggestion, not a requirement:

```text
cmd/
  tt/
internal/
  cli/
  config/
  database/
  entries/
  reports/
  timeutil/
migrations/
tests/
README.md
SPEC.md
```

Keep command parsing, business rules, persistence, and formatting separate.
Business logic such as overlap detection and report aggregation should be
testable without invoking the CLI.


