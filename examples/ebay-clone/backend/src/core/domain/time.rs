use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Timestamp(i64);

#[derive(Debug, Error)]
pub enum TimeError {
    #[error("timestamp must be a valid Unix milliseconds since UTC epoch")]
    InvalidTimestamp,
}

impl TryFrom<i64> for Timestamp {
    type Error = TimeError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        // Assuming all positive values are valid Unix timestamps in milliseconds
        if value >= 0 {
            Ok(Timestamp(value))
        } else {
            Err(TimeError::InvalidTimestamp)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct DurationMs(i64);

impl TryFrom<i64> for DurationMs {
    type Error = TimeError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        // Assuming all non-negative values are valid durations in milliseconds
        if value >= 0 {
            Ok(DurationMs(value))
        } else {
            Err(TimeError::InvalidTimestamp)
        }
    }
}

// docs/specs/ebay-spec-025 specifies the validation rules for Timestamp and DurationMs