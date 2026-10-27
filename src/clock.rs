use jiff::Timestamp;

use crate::error::{Result, validation};

/// Source of "now". Commands take a `Clock` instead of calling
/// `Timestamp::now()` directly so tests can pin the current instant.
pub trait Clock {
    fn now(&self) -> Timestamp;
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Timestamp::now()
    }
}

/// A clock frozen at a fixed instant, used by unit and integration tests.
pub struct FixedClock(pub Timestamp);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// Build the clock for a CLI run. `TT_NOW`, if set to an RFC 3339 timestamp,
/// freezes the current instant; this is the deterministic hook integration
/// tests use to drive `start`, `stop`, and the default report period.
pub fn from_env() -> Result<Box<dyn Clock>> {
    match std::env::var("TT_NOW") {
        Ok(raw) if !raw.trim().is_empty() => {
            let ts: Timestamp = raw.trim().parse().map_err(|e| {
                validation(format!("TT_NOW is not a valid RFC 3339 timestamp: {e}"))
                    .with_hint("example: TT_NOW=2026-10-27T13:30:00Z")
            })?;
            Ok(Box::new(FixedClock(ts)))
        }
        _ => Ok(Box::new(SystemClock)),
    }
}
