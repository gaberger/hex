use serde::{Serialize, Deserialize};
use std::convert::TryFrom;
use thiserror::Error;

// docs/specs/ebay-spec-025

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

impl TryFrom<u64> for ListingId {
    type Error = ListingIdValidationError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            return Err(ListingIdValidationError::ZeroValue);
        }
        Ok(ListingId(value))
    }
}

#[derive(Debug, Error)]
pub enum ListingIdValidationError {
    #[error("ListingId cannot be zero")]
    ZeroValue,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct BidId(pub u64);

impl TryFrom<u64> for BidId {
    type Error = BidIdValidationError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            return Err(BidIdValidationError::ZeroValue);
        }
        Ok(BidId(value))
    }
}

#[derive(Debug, Error)]
pub enum BidIdValidationError {
    #[error("BidId cannot be zero")]
    ZeroValue,
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy, Serialize, Deserialize)]
pub struct AuctionId(pub u64);

impl TryFrom<u64> for AuctionId {
    type Error = AuctionIdValidationError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        if value == 0 {
            return Err(AuctionIdValidationError::ZeroValue);
        }
        Ok(AuctionId(value))
    }
}

#[derive(Debug, Error)]
pub enum AuctionIdValidationError {
    #[error("AuctionId cannot be zero")]
    ZeroValue,
}