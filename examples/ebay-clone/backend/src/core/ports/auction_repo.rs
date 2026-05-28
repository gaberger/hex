use crate::core::domain::*;

#[async_trait]
pub trait AuctionRepoPort: Send + Sync {
    /// Fetch an auction by its unique identifier.
    async fn get_auction(&self, auction_id: &AuctionId) -> Result<Option<Auction>, RepositoryError>;

    /// List all auctions currently active.
    async fn list_active_auctions(&self) -> Result<Vec<Auction>, RepositoryError>;

    /// List auctions that ended within a specific time frame.
    async fn list_recently_ended_auctions(
        &self,
        start_time: UnixTimestampMillis,
        end_time: UnixTimestampMillis,
    ) -> Result<Vec<Auction>, RepositoryError>;
}

// DTOs for AuctionRepoPort
#[derive(Debug, Clone)]
pub struct Auction {
    pub id: AuctionId,
    pub listing_id: ListingId,
    pub current_bid: Option<Bid>,
    pub start_time: UnixTimestampMillis,
    pub end_time: UnixTimestampMillis,
    // Add other fields as per auction domain model
}

// Example of how this trait might be used in a reducer or service layer
// hex-core/src/services/auction_service.rs

#[derive(Debug, Clone)]
pub struct Bid {
    pub id: BidId,
    pub bidder_id: UserId,
    pub amount: Money,
    pub timestamp: UnixTimestampMillis,
}

// This is a marker for the domain types used in the auction repository.
// Refer to docs/specs/ebay-spec-019 for more details on the Auction and Bid models.