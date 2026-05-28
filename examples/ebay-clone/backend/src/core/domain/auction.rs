use serde::{Deserialize, Serialize};
use chrono::DateTime;
use crate::core::domain::error::DomainError;
use crate::core::domain::user::UserIdentity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuctionStatus {
    Active,
    Closed,
    Unsold,
}

impl AuctionStatus {
    pub fn is_active(&self) -> bool {
        *self == AuctionStatus::Active
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Auction {
    pub current_highest_cents: u32,
    pub current_highest_bidder: Option<UserIdentity>,
    pub end_time: DateTime<chrono::Utc>,
    pub status: AuctionStatus,
    pub winner_identity: Option<UserIdentity>,
    pub winning_amount_cents: Option<u32>,
}

impl Auction {
    pub fn new(
        starting_price_cents: u32,
        end_time: DateTime<chrono::Utc>,
    ) -> Result<Self, DomainError> {
        if starting_price_cents == 0 {
            return Err(DomainError::InvalidStartingPrice);
        }
        Ok(Auction {
            current_highest_cents: starting_price_cents,
            current_highest_bidder: None,
            end_time,
            status: AuctionStatus::Active,
            winner_identity: None,
            winning_amount_cents: None,
        })
    }

    pub fn place_bid(&mut self, bidder: UserIdentity, bid_amount_cents: u32) -> Result<(), DomainError> {
        if self.status != AuctionStatus::Active {
            return Err(DomainError::AuctionEnded);
        }
        if bid_amount_cents <= self.current_highest_cents {
            return Err(DomainError::BidTooLow);
        }
        // Assuming bidder validation and other checks are handled elsewhere
        self.current_highest_bidder = Some(bidder);
        self.current_highest_cents = bid_amount_cents;
        Ok(())
    }

    pub fn end_auction(&mut self) {
        if self.status == AuctionStatus::Active {
            self.status = if let Some(winner) = &self.current_highest_bidder {
                AuctionStatus::Closed
            } else {
                AuctionStatus::Unsold
            };
            self.winner_identity = self.current_highest_bidder.clone();
            self.winning_amount_cents = Some(self.current_highest_cents);
        }
    }

    pub fn is_ended(&self) -> bool {
        matches!(self.status, AuctionStatus::Closed | AuctionStatus::Unsold)
    }
}

// docs/specs/ebay-spec-012