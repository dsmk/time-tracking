# Live verification checklist

`docs/opentodo-compatibility.md` is written from OpenTodo's documentation and
verified against a mock `ot`. This file is the procedure for closing the gap:
run it against a real `ot` client and a real OpenTodo instance, then record the
results in §11 of the compatibility document (replacing the **UNVERIFIED**
tags).

Use a scratch account or workspace. Several steps write data.

`testing/README.md` sets up a throwaway server in a container with sample data.
Much of what follows is automated once that exists:
`testing/capture-findings.sh` covers §1, and `testing/run-live-tests.sh` covers
most of §2, §3, §4, §6 and §7. The steps that still need a human are the web
interface checks in §2 and the offline tests in §5.

## 0. Prerequisites

```bash
ot version --json
ot auth status --json
tt --version
```

## 1. Offline facts, no server needed

`ot schema` is documented to need no configuration or server.

```bash
ot schema > /tmp/ot-schema.json
jq '.commands[] | select(.name | startswith("time"))' /tmp/ot-schema.json
jq '.schemas | keys' /tmp/ot-schema.json
```

Record:

- [ ] The exact flags of `time start`, `time stop`, `time list`.
- [ ] The output schema of each — in particular whether `time list` returns a
      bare array or `{entries, nextCursor, serverTime}` (compat §3, open item 1).
- [ ] The output schema of `time start` and `time stop` (open item 2).
- [ ] The default and maximum `--limit` on `time list` (open item 4).
- [ ] The schema of `list` (tasks), for the `tt start` picker (open item 8).
- [ ] Which key in `version --json` holds the client version (open item 3).

If `time list` turns out to paginate, `tt` must follow the cursor instead of
warning about it: see `App.Fetch` in `internal/commands/app.go`.

## 2. Round trip through the web interface

```bash
ot add "tt live check" --json        # note the task id, TASK
tt start "live check note" --task TASK
tt status
```

- [ ] The OpenTodo **web interface** shows a running timer on that task.
- [ ] The note matches.
- [ ] The start time matches to the minute.

```bash
tt stop
tt log --today --format json | jq '.[0]'
```

- [ ] The completed entry appears in the web interface with the same duration.
- [ ] `tt log` and the web interface agree on the minutes.
- [ ] Satisfies `SPEC.md` §12.2.

## 3. Timezone interpretation (open item 5)

Create an entry close to local midnight (the Time tab in the web app can add
one at an explicit time), then:

```bash
tt log --from YYYY-MM-DD --to YYYY-MM-DD --format json
ot time list --from YYYY-MM-DD --to YYYY-MM-DD --json
```

- [ ] `tt` includes the entry on the correct local day for the configured
      `timezone`.
- [ ] Compare the raw `ot` result with `tt`'s to see which zone `ot` applied to
      `--from`/`--to`. If `ot` uses a zone more than 24 hours from the
      configured one — it should not — widen `Period::query_window`.

## 4. Error paths

- [ ] **Stop with nothing running:** `ot time stop; echo $?` → confirm exit 1
      and note the error `code` (open item 7). `tt stop` must exit 8 without
      calling `ot time stop`.
- [ ] **Read-only token:** `OPENTODO_TOKEN=<read token> tt start ... --task T`
      → exit 5, `insufficient_scope`.
- [ ] **Read-only mode:** `OPENTODO_READ_ONLY=1 tt start ... --task T` → exit 5.
- [ ] **Unreachable:** point `OPENTODO_URL` at a dead host → `tt status` exits 4
      and says `tt` cannot record locally.
- [ ] **Bad task id:** `tt start x --task zzzz` → exit 1, `not_found`.
- [ ] No message in any of the above contains a token.

## 5. Offline behaviour (`SPEC.md` §7 and §11)

The CLI docs say the `ot` client is online-only, so the expected result is
clean failure, **not** queued work.

1. [ ] Disconnect the network.
2. [ ] `tt status` → exit 4, "server could not be reached".
3. [ ] `tt start "offline test" --task TASK` → exit 4, and **no** entry is
       created anywhere.
4. [ ] `tt log --today` → exit 4 (there is no local cache to read).
5. [ ] Reconnect. `tt log --today` shows no phantom entry from step 3.
6. [ ] The OpenTodo web interface shows no entry from step 3.

If any of these instead succeeds offline, the compatibility document's §5 is
wrong and must be rewritten before any offline support is claimed.

## 6. Sync visibility

1. [ ] `tt sync status` while online → "unavailable", server reachable.
2. [ ] `tt sync status` while offline → "offline".
3. [ ] `tt sync` → exit 6 with guidance, and `strace`/`ps` shows no `ot sync`
       attempt.
4. [ ] Start a timer on a phone or in the web app; `tt status` sees it once
       that device has synced. Note roughly how long that takes.

## 7. Multi-device and conflict behaviour

1. [ ] Start a timer with `tt`, then stop it in the web interface. `tt stop`
       afterwards must exit 8 (nothing running), not report a false stop.
2. [ ] Start a timer in the web app, then run `tt start --task OTHER`. `tt`
       must refuse without `--switch`.
3. [ ] With `--switch`, confirm OpenTodo ended the first entry and that no time
       was lost or double-counted.
4. [ ] Run `tt start` twice quickly for the same task and check whether two
       entries appear. `--idempotency-key` is not documented for
       `time start` (compat §7); record what actually happens.

## 8. Record the results

Update `docs/opentodo-compatibility.md`:

- Replace each **UNVERIFIED** tag with what you observed.
- Add the tested `ot` version and the OpenTodo server version.
- Add the date and who ran it.
- Delete items from §11 as they are closed.
