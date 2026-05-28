use serde::{Serialize, Deserialize};
use std::convert::TryFrom;
use thiserror::Error;

#[derive(Debug, PartialEq, Eq, Hash, Clone, Serialize, Deserialize)]
pub struct UserId(pub String);

impl TryFrom<String> for UserId {
    type Error = UserIdValidationError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.len() != 26 || !value.chars().all(|c| c.is_alphanumeric()) {
            return Err(UserIdValidationError::InvalidFormat);
        }
        Ok(UserId(value))
    }
}

#[derive(Debug, Error)]
pub enum UserIdValidationError {
    #[error("UserId must be a 26-character alphanumeric string")]
    InvalidFormat,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct ListingId(pub u64);

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct BidId(pub u64);

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct AuctionId(pub u64);

// Spec references: docs/specs/ebay-spec-003, docs/specs/ebay-spec-025
#[derive(Debug, PartialEq, Eq, Clone, Copy, Serialize, Deserialize)]
pub struct Money(pub u64); // Cents

impl TryFrom<u64> for Money {
    type Error = MoneyValidationError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            return Err(MoneyValidationError::ZeroValue);
        }
        Ok(Money(value))
    }
}

#[derive(Debug, Error)]
pub enum MoneyValidationError {
    #[error("Money cannot be zero")]
    ZeroValue,
}