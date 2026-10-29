//! `tt start`, `tt stop`, `tt status`.

use std::io::{BufRead, IsTerminal, Write};

use jiff::ToSpan;

use super::App;
use crate::cli::{JsonFlag, StartArgs};
use crate::error::{Error, Kind, Result, usage};
use crate::ot::EntryFilter;
use crate::ot::model::TimeEntry;
use crate::output;

pub fn start(app: &App, args: &StartArgs) -> Result<()> {
    let task = resolve_task(app, args.task.as_deref())?;

    // OpenTodo ends the current entry when another task is started, so tt makes
    // that explicit rather than letting a typo silently close an entry.
    if let Some(running) = app.ot.get_running_timer()? {
        if !args.switch {
            return Err(Error::new(
                Kind::Usage,
                format!(
                    "a timer is already running: {} (since {})",
                    running.label(),
                    running.started_at.to_zoned(app.tz.clone()).strftime("%H:%M")
                ),
            )
            .with_hint("stop it with `tt stop`, or pass --switch to let OpenTodo end it"));
        }
        output::warn(&format!(
            "OpenTodo will end the running entry for {} before starting the new one",
            running.label()
        ));
    }

    let started = app.ot.start_timer(&task, args.note.as_deref())?;
    // `ot time start --json` output is undocumented: when it tells us nothing,
    // ask OpenTodo what is running rather than reporting a made-up entry.
    let entry = match started {
        Some(entry) => Some(entry),
        None => app.ot.get_running_timer()?,
    };

    match (entry, args.json) {
        (Some(entry), true) => output::print_json(&output::entry_json(&entry, &app.tz, app.now())),
        (Some(entry), false) => {
            println!(
                "Started {} at {}",
                entry.label(),
                entry.started_at.to_zoned(app.tz.clone()).strftime("%H:%M %Z")
            );
            if let Some(id) = &entry.task_id {
                println!("  task {id}  entry {}", entry.id);
            }
            Ok(())
        }
        (None, true) => output::print_json(&serde_json::json!({
            "started": true,
            "task": task,
            "entry": serde_json::Value::Null,
            "warning": "OpenTodo accepted the start but returned no entry details",
        })),
        (None, false) => {
            println!("OpenTodo accepted the start for task {task}.");
            output::warn("OpenTodo returned no entry details; check `tt status`");
            Ok(())
        }
    }
}

pub fn stop(app: &App, args: &JsonFlag) -> Result<()> {
    // Check before writing: `ot time stop` uses exit 1 both for "nothing runs"
    // and for a server failure, and a failed write is worth not sending at all.
    let running = app.ot.get_running_timer()?.ok_or_else(|| {
        Error::new(Kind::NoTimer, "no timer is running, so nothing was stopped")
            .with_hint("start one with `tt start --task <TASK_ID>`")
    })?;

    let stopped = app.ot.stop_timer()?;
    let entry = match stopped {
        Some(entry) => Some(entry),
        // Undocumented output: re-read the entry instead of computing a
        // duration tt cannot verify.
        None => refetch(app, &running)?,
    };

    match (entry, args.json) {
        (Some(entry), true) => output::print_json(&output::entry_json(&entry, &app.tz, app.now())),
        (Some(entry), false) => {
            println!("Stopped {}", entry.label());
            println!("  {}", output::entry_line(&entry, &app.tz, app.now()));
            Ok(())
        }
        (None, true) => output::print_json(&serde_json::json!({
            "stopped": true,
            "entryId": running.id,
            "entry": serde_json::Value::Null,
            "warning": "OpenTodo accepted the stop but returned no entry details",
        })),
        (None, false) => {
            println!("Stopped the timer for {}.", running.label());
            output::warn(
                "OpenTodo returned no entry details, so the recorded duration is not shown; \
                 check `tt log --today`",
            );
            Ok(())
        }
    }
}

pub fn status(app: &App, args: &JsonFlag) -> Result<()> {
    let running = app.ot.get_running_timer()?;
    let now = app.now();
    match (running, args.json) {
        (Some(entry), true) => output::print_json(&serde_json::json!({
            "running": true,
            "entry": output::entry_json(&entry, &app.tz, now),
        })),
        (Some(entry), false) => {
            let elapsed = crate::report::format_duration(output::entry_seconds(&entry, now));
            println!(
                "Running: {} for {} (since {})",
                entry.label(),
                elapsed,
                entry.started_at.to_zoned(app.tz.clone()).strftime("%H:%M %Z")
            );
            if let Some(id) = &entry.task_id {
                println!("  task {id}  entry {}", entry.id);
            }
            Ok(())
        }
        (None, true) => {
            output::print_json(&serde_json::json!({ "running": false, "entry": null }))
        }
        (None, false) => {
            println!("No timer is running.");
            Ok(())
        }
    }
}

/// Re-read an entry OpenTodo did not describe, by listing the days around it.
fn refetch(app: &App, running: &TimeEntry) -> Result<Option<TimeEntry>> {
    let started = running.started_at.to_zoned(app.tz.clone()).date();
    let from = started.checked_sub(1.day()).unwrap_or(started);
    let to = started.checked_add(1.day()).unwrap_or(started);
    let filter = EntryFilter {
        from: Some(from),
        to: Some(to),
        task: running.task_id.clone(),
        running: false,
        limit: Some(100),
    };
    let page = app.ot.list_time_entries(&filter)?;
    Ok(page.entries.into_iter().find(|e| e.id == running.id))
}

/// Find the task to track against: the flag, or an interactive choice.
///
/// OpenTodo attaches every time entry to a task and documents no way to start
/// one without a task, so there is no "just start something" path here.
fn resolve_task(app: &App, explicit: Option<&str>) -> Result<String> {
    if let Some(task) = explicit {
        return Ok(task.to_string());
    }
    if !(std::io::stdin().is_terminal() && std::io::stderr().is_terminal()) {
        return Err(usage("no task was given, and tt cannot prompt when it is not run from a terminal")
            .with_hint("pass --task <TASK_ID>; `ot list --json` shows the available tasks"));
    }

    let tasks = app.ot.list_tasks()?;
    if tasks.is_empty() {
        return Err(usage("OpenTodo returned no tasks to choose from")
            .with_hint("create one with `ot add \"...\"`, then pass --task"));
    }

    // Prompts go to stderr so stdout stays clean for piped output.
    let shown = tasks.iter().take(20).collect::<Vec<_>>();
    eprintln!("Select a task:");
    for (i, task) in shown.iter().enumerate() {
        let project = task.project_name.as_deref().map(|p| format!(" ({p})")).unwrap_or_default();
        eprintln!("  {:>2}. {}{}  [{}]", i + 1, task.title.as_deref().unwrap_or("(untitled)"), project, task.id);
    }
    if tasks.len() > shown.len() {
        eprintln!("  ... and {} more; pass --task to choose one of those", tasks.len() - shown.len());
    }
    eprint!("Number: ");
    let _ = std::io::stderr().flush();

    let mut line = String::new();
    std::io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|e| usage(format!("could not read your choice: {e}")))?;
    let choice: usize = line
        .trim()
        .parse()
        .map_err(|_| usage(format!("{:?} is not one of the listed numbers", line.trim())))?;
    let task = shown
        .get(choice.wrapping_sub(1))
        .ok_or_else(|| usage(format!("{choice} is not one of the listed numbers")))?;
    Ok(task.id.clone())
}
