use serde::{Deserialize, Serialize};

/// Represents a bid in an auction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bid {
    /// The unique identifier of the user placing the bid.
    pub bidder_id: String,

    /// The amount of the bid in cents.
    pub amount_cents: u64,
}

// ADR-2026-05-19-0721
impl Bid {
    /// Creates a new `Bid`.
    ///
    /// # Arguments
    /// * `bidder_id` - The unique identifier of the user placing the bid.
    /// * `amount_cents` - The amount of the bid in cents.
    pub fn new(bidder_id: String, amount_cents: u64) -> Self {
        Bid { bidder_id, amount_cents }
    }
}