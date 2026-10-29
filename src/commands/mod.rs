pub mod entries;
pub mod reports;
pub mod sync;
pub mod timer;

use jiff::civil::Date;
use jiff::tz::TimeZone;
use jiff::Timestamp;

use crate::clock::Clock;
use crate::config::Config;
use crate::error::{Result, usage};
use crate::ot::model::TimeEntry;
use crate::ot::{EntryFilter, OpenTodo};
use crate::report::Period;

/// Everything a command needs: resolved settings, a clock, and the adapter.
pub struct App {
    pub config: Config,
    pub tz: TimeZone,
    pub clock: Box<dyn Clock>,
    pub ot: Box<dyn OpenTodo>,
}

/// Entries plus the metadata a report needs to stay honest about them.
pub struct Fetched {
    pub entries: Vec<TimeEntry>,
    /// OpenTodo's clock, when it reported one; otherwise tt's.
    pub now: Timestamp,
    pub warnings: Vec<String>,
    /// OpenTodo may have had more entries than it returned, so any total built
    /// from these is a lower bound.
    pub incomplete: bool,
}

impl App {
    pub fn now(&self) -> Timestamp {
        self.clock.now()
    }

    pub fn timezone_label(&self) -> String {
        crate::report::tz_label(&self.tz)
    }

    pub fn today(&self) -> Date {
        self.now().to_zoned(self.tz.clone()).date()
    }

    /// Fetch every entry that could touch `period`.
    ///
    /// The query window is widened by a day on each side because `ot` applies
    /// its own timezone to `--from`/`--to` (docs/opentodo-compatibility.md §9);
    /// callers clip to the real boundaries themselves.
    pub fn fetch(&self, period: Period, task: Option<String>, limit: u32) -> Result<Fetched> {
        let window = period.query_window()?;
        let filter = EntryFilter {
            from: Some(window.from),
            to: Some(window.to),
            task,
            running: false,
            limit: Some(limit),
        };
        let page = self.ot.list_time_entries(&filter)?;

        let mut warnings = Vec::new();
        let mut incomplete = false;
        if page.entries.len() as u64 >= u64::from(limit) {
            incomplete = true;
            warnings.push(format!(
                "OpenTodo returned {} entries, which is the requested limit, so this report may \
                 be incomplete; re-run with a larger --limit",
                page.entries.len()
            ));
        }
        if page.next_cursor.is_some() {
            incomplete = true;
            warnings.push(
                "OpenTodo reported more entries beyond this page and the ot CLI has no \
                 documented pagination flag, so this report may be incomplete"
                    .to_string(),
            );
        }

        let mut entries = page.entries;
        entries.sort_by_key(|e| (e.started_at, e.id.clone()));

        Ok(Fetched {
            entries,
            now: page.server_time.unwrap_or_else(|| self.now()),
            warnings,
            incomplete,
        })
    }
}

pub fn parse_date(raw: &str) -> Result<Date> {
    raw.trim().parse::<Date>().map_err(|e| {
        usage(format!("{raw:?} is not a date: {e}")).with_hint("use the form YYYY-MM-DD")
    })
}

pub fn resolve_limit(requested: Option<u32>, config: &Config) -> Result<u32> {
    match requested {
        Some(0) => Err(usage("--limit must be greater than zero")),
        Some(n) => Ok(n),
        None => Ok(config.max_entries),
    }
}
