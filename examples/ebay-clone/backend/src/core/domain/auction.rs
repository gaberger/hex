use serde::{Deserialize, Serialize};
use std::time::SystemTime;
use uuid::Uuid;

// ADR-2026-05-19-0721
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuctionStatus {
    Active,
    Closed,
    Unsold,
}

impl AuctionStatus {
    pub fn is_active(&self) -> bool {
        matches!(self, AuctionStatus::Active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Auction {
    pub current_highest_cents: u64,
    pub current_highest_bidder: Option<Uuid>,
    pub end_time: SystemTime,
    pub status: AuctionStatus,
    pub winner_identity: Option<Uuid>,
    pub winning_amount_cents: Option<u64>,
}

impl Auction {
    pub fn new(end_time: SystemTime) -> Self {
        Auction {
            current_highest_cents: 0,
            current_highest_bidder: None,
            end_time,
            status: AuctionStatus::Active,
            winner_identity: None,
            winning_amount_cents: None,
        }
    }

    pub fn place_bid(&mut self, bidder_id: Uuid, bid_amount_cents: u64) -> Result<(), DomainError> {
        if !self.status.is_active() {
            return Err(DomainError::AuctionEnded);
        }
        if bid_amount_cents <= self.current_highest_cents {
            return Err(DomainError::BidTooLow);
        }

        self.current_highest_bidder = Some(bidder_id);
        self.current_highest_cents = bid_amount_cents;

        Ok(())
    }

    pub fn end_auction(&mut self) -> Result<(), DomainError> {
        if !self.status.is_active() {
            return Err(DomainError::AuctionEnded);
        }
        self.status = AuctionStatus::Closed;
        self.winner_identity = self.current_highest_bidder;
        self.winning_amount_cents = Some(self.current_highest_cents);

        Ok(())
    }
}

// docs/specs/ebay-spec-017
#[derive(Debug, Clone)]
pub struct Bid {
    pub bidder_id: Uuid,
    pub amount_cents: u64,
    pub timestamp: SystemTime,
}

// docs/specs/ebay-spec-018
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("Username is already taken")]
    UsernameTaken,
    #[error("Invalid username provided")]
    InvalidUsername,
    #[error("Title length is invalid")]
    InvalidTitleLength,
    #[error("Starting price is invalid")]
    InvalidStartingPrice,
    #[error("Duration is invalid")]
    InvalidDuration,
    #[error("Bid amount is too low")]
    BidTooLow,
    #[error("Auction has ended")]
    AuctionEnded,
    #[error("Self-bid is forbidden")]
    SelfBidForbidden,
}