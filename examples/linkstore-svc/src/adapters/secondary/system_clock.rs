//! The real wall clock. The only place this application asks the machine what
//! time it is.

use crate::ports::{Clock, TimestampValue};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> TimestampValue {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| since.as_millis())
            .unwrap_or(0);
        // `try_from`, never `as`. A silent truncation here would write a
        // wrong date and nothing would complain.
        TimestampValue::from_millis(i64::try_from(millis).unwrap_or(i64::MAX))
    }
}
