# tt — local-first command-line time tracker

A fast, text-oriented time tracker inspired by [klog](https://klog.jotaen.net).
Everything lives in a local SQLite database; no network access is ever needed.

Built to [`SPEC.md`](SPEC.md). This is the MVP: tracking, entry management, and
reporting on one machine. There is no server, sync, daemon, or GUI — but the
records carry UUIDs, revisions, and deletion tombstones so synchronization can
be added later without reshaping the data.

## Build

```sh
cargo build --release     # binary at target/release/tt
cargo test                # 104 unit and integration tests
```

SQLite is compiled in, so there is no system database dependency.

## Quick start

```sh
tt start "Implement login" --project app --tag focus
tt status
tt stop

tt add "Code review" --start 2026-10-27T09:00 --duration 90m --project alpha
tt log --today
tt report week
```

## Commands

| Command | Purpose |
|---|---|
| `tt init` | Create the database and apply migrations (safe to repeat) |
| `tt start DESC` | Start a timer — `--project`, `--tag`, `--at`, `--allow-overlap` |
| `tt stop` | Stop the running timer — `--at`, `--allow-overlap` |
| `tt status` | Show the running timer — `--json` |
| `tt add DESC` | Record a completed entry — `--start` plus `--end` or `--duration` |
| `tt log` | List entries, newest first — filters below |
| `tt show ID` | Show one entry — `--json`, `--include-deleted` |
| `tt edit ID` | Change an entry — see below |
| `tt delete ID` | Soft-delete an entry — `--yes` |
| `tt report week` | Weekly summary — `--week`, `--project`, `--tag`, `--format` |
| `tt report range` | Date-range summary — `--from`, `--to`, `--project`, `--tag`, `--format` |
| `tt config show\|set\|path` | Inspect or change configuration |

`tt log` filters: `--today`, `--from DATE`, `--to DATE`, `--project`, `--tag`,
`--search TEXT`, `--limit N`, `--json`. `--today` cannot be combined with
`--from` or `--to`. A date range is inclusive, and an entry is included if any
part of it falls inside.

`tt edit` options: `--description`, `--project` / `--clear-project`, `--start`,
`--end`, `--tag` (replaces all tags), `--add-tag`, `--remove-tag`,
`--allow-overlap`. At least one is required. Edits are validated against the
resulting entry as a whole; a failed edit changes nothing.

Every command supports `--help`. Shell completion (`tt completion`, optional in
the spec) is not implemented.

## Tags

Tags can be given either way, and the two merge:

```sh
tt start "Investigate alert" --tag incident --tag oncall
tt start "Investigate #incident while #on-call"
```

A `#tag` written in the description is mirrored into the entry's tag list; the
`#` text stays in the description. Duplicates are detected case-insensitively
and the first spelling entered is kept for display. `tt edit --description`
likewise adds any `#tags` in the new text, and an explicit `--remove-tag` wins
over them.

## Files and precedence

| What | Default location |
|---|---|
| Database | `$XDG_DATA_HOME/tt/tt.db`, else `~/.local/share/tt/tt.db` |
| Configuration | `$XDG_CONFIG_HOME/tt/config.toml`, else `~/.config/tt/config.toml` |

macOS and Windows use their own standard application-data directories.

If `timezone` is unset, the system timezone is used. Its IANA name is recovered
from `$TZ` or `/etc/timezone` when the operating system does not report one
directly (common in containers); if no name can be determined, times are still
correct but a warning suggests setting `timezone` explicitly, since each entry
records the zone it was created in.

- **Database path:** `--db PATH` wins, then `$TT_DB`, then the default.
- **Config path:** `$TT_CONFIG` wins, then the default.

The database is created on first use; `tt init` is optional. Opening an
existing database never deletes or overwrites it.

## Configuration

`config.toml`, every setting optional:

```toml
timezone = "America/New_York"   # default: the system timezone
week_start = "monday"           # default: monday; any weekday
duration_precision = "minute"   # default: minute; or "second"
default_project = ""            # default: none
```

Set values with `tt config set timezone America/New_York`, which validates
before writing and leaves other keys (including unrecognized ones) intact. An
invalid setting is a fatal error naming the setting and the file; an unknown
key is ignored with a warning on stderr.

## Time handling

**Accepted input.** `--at`, `--start`, and `--end` take:

| Form | Meaning |
|---|---|
| `2026-10-27T09:30` | Local time in the configured timezone (seconds optional) |
| `2026-10-27 09:30` | The same, with a space separator |
| `2026-10-27T09:30-04:00` | An explicit offset — unambiguous |
| `2026-10-27T13:30:00Z` | UTC — unambiguous |

`--from` and `--to` take calendar dates, `YYYY-MM-DD`, in the configured
timezone.

**Daylight saving.** A local time without an offset that is *ambiguous* (it
occurs twice) or *nonexistent* (it was skipped) is rejected with an explanation
and a suggested replacement you can paste back verbatim. Supply an explicit
offset to say which you meant.

**Durations.** Integers with `h`, `m`, `s` units, combinable: `45s`, `90m`,
`1h30m`, `1h30m15s`.

**Storage and display.** Instants are stored as UTC RFC 3339 strings at whole-
second precision (`2026-10-27T13:30:00Z`) — one format, never mixed. Display
and all calendar reasoning use the configured timezone, in the format
`YYYY-MM-DD HH:MM` (gaining `:SS` under `duration_precision = "second"`).
Durations render as `1h 30m` (`1h 30m 15s` at second precision). Rounding is
applied only when rendering text; stored instants and report aggregates are
never rounded.

Because durations are elapsed time between real instants, a day containing a
daylight-saving transition can legitimately total 23 or 25 hours.

## Overlap

At most one timer runs at a time, enforced by a database constraint rather
than by a check alone — two CLI processes racing to start a timer cannot both
win. Entries may not overlap each other unless `--allow-overlap` is given; a
running timer counts as occupying all time from its start onward. Overlaps that
were explicitly allowed remain visible: reports list the overlapping pairs and
warn that time is counted more than once.

## Output

Results go to stdout, diagnostics and warnings to stderr. `--json` and
`--format json` emit nothing but the JSON document, so they are safe to pipe.

- **JSON** uses stable field names, full UUIDs, integer-second durations,
  UTC timestamps ending in `Z` alongside local ones with an explicit offset,
  and deterministic array order. `tt status --json` prints `null` when no timer
  is running.
- **CSV** (reports only) is long-format: `period_start,period_end,group_type,
  group_name,seconds`, with `group_type` one of `total`, `project`, `day`, `tag`.
- **Text** list output shows a short 8-character id; `show` and JSON give the
  full UUID.

### Exit status

| Code | Meaning |
|---|---|
| 0 | Success |
| 2 | Invalid input (bad times, durations, flags, or a missing confirmation) |
| 3 | Entry not found |
| 4 | Conflict (overlap, a timer already running, an ambiguous id prefix) |
| 5 | Configuration error |
| 6 | Database error |

## Data model

`entries` holds `id` (UUID), `start_at`, `end_at` (null while running),
`description`, `project`, `created_at`, `updated_at`, `revision`, `deleted_at`,
and the `timezone` in effect when the entry was created. `entry_tags` holds
`(entry_id, tag)`, unique per entry case-insensitively.

Deleting is a soft delete: `deleted_at` is set, the revision increments, and
the row stays as a tombstone. Normal listings and reports exclude it;
`tt show --include-deleted` can still inspect it.

Schema changes are applied as numbered migrations inside a transaction and
recorded in `schema_migrations`.

## Testing

```sh
cargo test
```

Unit tests cover duration and timestamp parsing, timezone conversion, week
boundaries for every week-start day, range clipping, overlap detection, tag
normalization, and report aggregation. Integration tests drive the real binary
against a temporary database: the start/stop lifecycle across separate
processes, rejected second timers, filters, edits, soft deletion, reports, JSON
and CSV output, failed commands leaving the database untouched, and eight
concurrent `tt start` invocations producing exactly one running timer.

Tests that depend on the current time pin it with the `TT_NOW` environment
variable, which accepts an RFC 3339 timestamp and freezes "now" for a single
invocation. It exists for deterministic testing, not for recording time.

## Layout

```text
src/
  main.rs        entry point, error reporting, exit codes
  cli.rs         argument parsing and command dispatch
  config.rs      TOML configuration and path resolution
  db.rs          connection setup and schema migrations
  model.rs       Entry type, description/project/tag normalization
  entries.rs     persistence and business rules (overlap, lifecycle, editing)
  reports.rs     period calculation and aggregation
  timeutil.rs    parsing, formatting, timezone and week arithmetic
  output.rs      text, JSON, and CSV rendering
  clock.rs       injectable clock
tests/           integration tests driving the binary
```

Business rules are testable without invoking the CLI: `entries` and `reports`
take plain values and a `Connection`, and know nothing about argument parsing
or formatting.
