use jiff::Timestamp;

use crate::error::{Result, usage};

/// Source of "now". Commands take a `Clock` rather than calling
/// `Timestamp::now()` so tests can pin the current instant.
pub trait Clock {
    fn now(&self) -> Timestamp;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

pub struct FixedClock(pub Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// Build the clock for a run. `TT_NOW`, when set to an RFC 3339 timestamp,
/// freezes the current instant; the integration tests use it to make default
/// report periods and elapsed times deterministic.
pub fn from_env() -> Result<Box<dyn Clock>> {
    match std::env::var("TT_NOW") {
        Ok(raw) if !raw.trim().is_empty() => {
            let ts: Timestamp = raw.trim().parse().map_err(|e| {
                usage(format!("TT_NOW is not a valid RFC 3339 timestamp: {e}"))
                    .with_hint("example: TT_NOW=2026-10-27T13:30:00Z")
            })?;
            Ok(Box::new(FixedClock(ts)))
        }
        _ => Ok(Box::new(SystemClock)),
    }
}
