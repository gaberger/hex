use crate::core::domain::*;
use async_trait::async_trait;

/// AuctionRepoPort defines read-only operations on auctions.
///
/// Returns the domain `Auction` type directly — the domain is the source of
/// truth (hex rule). This port previously redefined its own `Auction`/`Bid`
/// DTOs, which collided with the domain re-exports in any outer file that
/// globbed both `core::domain::*` and `core::ports::*`. Removed so adapters
/// conform inward to the domain types.
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