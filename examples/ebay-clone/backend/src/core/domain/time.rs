use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Timestamp(i64);

#[derive(Debug, Error)]
pub enum TimeValidationError {
    #[error("Timestamp must be in the past or present")]
    InvalidTimestamp,
}

impl TryFrom<i64> for Timestamp {
    type Error = TimeValidationError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value <= chrono::Utc::now().timestamp_millis() {
            Ok(Timestamp(value))
        } else {
            Err(TimeValidationError::InvalidTimestamp)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DurationMs(i64);

impl TryFrom<i64> for DurationMs {
    type Error = TimeValidationError;

    fn try_from(value: i64) -> Result<Self, Self::Error> {
        if value >= 0 {
            Ok(DurationMs(value))
        } else {
            Err(TimeValidationError::InvalidTimestamp)
        }
    }
}

// ADR-2026-05-19-0721
// docs/specs/ebay-spec-025