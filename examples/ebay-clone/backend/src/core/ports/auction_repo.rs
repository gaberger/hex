use crate::core::domain::*;

#[async_trait]
pub trait AuctionRepoPort: Send + Sync {
    /// Fetch an auction by its unique identifier.
    async fn get_auction(&self, auction_id: &AuctionId) -> Result<Option<Auction>, DomainError>;

    /// List all auctions currently active.
    async fn list_active_auctions(&self) -> Result<Vec<Auction>, DomainError>;

    /// List auctions that ended within a specific time frame.
    async fn list_recently_ended_auctions(
        &self,
        start_time: Timestamp,
        end_time: Timestamp,
    ) -> Result<Vec<Auction>, DomainError>;
}

// DTOs for AuctionRepoPort
#[derive(Debug, Clone)]
pub struct Auction {
    pub id: AuctionId,
    pub listing_id: ListingId,
    pub current_bid: Option<Bid>,
    pub start_time: Timestamp,
    pub end_time: Timestamp,
}

#[derive(Debug, Clone)]
pub struct Bid {
    pub id: BidId,
    pub bidder_id: UserId,
    pub amount: Money,
    pub timestamp: Timestamp,
}