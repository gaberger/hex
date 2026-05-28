use serde::{Deserialize, Serialize};
use chrono::NaiveDateTime;

// docs/specs/ebay-spec-012

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub id: String,
    pub title: String,
    pub description: String,
    pub starting_price_cents: u64,
    pub created_at: NaiveDateTime,
    pub auction_id: String,
}

impl Listing {
    // ADR-2026-05-19-0721
    pub fn new(
        id: String,
        title: String,
        description: String,
        starting_price_cents: u64,
        created_at: NaiveDateTime,
        auction_id: String,
    ) -> Result<Self, DomainError> {
        if title.len() < 3 || title.len() > 100 {
            return Err(DomainError::InvalidTitleLength);
        }
        if starting_price_cents == 0 {
            return Err(DomainError::InvalidStartingPrice);
        }

        Ok(Listing {
            id,
            title,
            description,
            starting_price_cents,
            created_at,
            auction_id,
        })
    }
}