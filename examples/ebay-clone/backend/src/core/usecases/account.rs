use crate::core::domain::{Bid, DomainError, ListingId, UserId};
use crate::core::ports::{
    BidRepoPort, PlaceBidInput, ReducerCallPort, WatchListingInput,
};

/// Account-centric use cases for an authenticated bidder.
///
/// The original draft called repository methods that the ports never declared
/// (`list_by_bidder`, `list_closed_by_winner`, `list_by_seller`), invoked the
/// async `ReducerCallPort` methods without awaiting them, passed the wrong
/// arity to `place_bid`, and referenced `DomainError` variants that don't
/// exist (`RepositoryAccessFailed`, `AuctionClosed`, `Unexpected`).
///
/// Ports are the contract and their adapter impls live outside this cluster,
/// so the use case is conformed to the ports — not the reverse:
///
/// * placing a bid / watching a listing go through `ReducerCallPort` with the
///   real input DTOs (`PlaceBidInput`, `WatchListingInput`), and are awaited;
/// * a bidder's bid history maps to `BidRepoPort::get_bids_by_user_id`.
///
/// "Won auctions" and "my listings" had no supporting port query (and the
/// domain `Listing` carries no seller field, nor does the auction repo DTO
/// expose a winner), so they are not exposed here. Adding them would require
/// new port methods plus matching adapter impls, which is out of scope for
/// this cluster and would break adapters not present here.
pub struct AccountUseCase {
    reducer: Box<dyn ReducerCallPort>,
    bid_repo: Box<dyn BidRepoPort>,
}

impl AccountUseCase {
    pub fn new(
        reducer: Box<dyn ReducerCallPort>,
        bid_repo: Box<dyn BidRepoPort>,
    ) -> Self {
        Self { reducer, bid_repo }
    }

    /// Place a bid on `listing_id` on behalf of `bidder`.
    pub async fn place_bid(
        &self,
        bidder: UserId,
        listing_id: ListingId,
        amount: u64,
    ) -> Result<Bid, DomainError> {
        self.reducer
            .place_bid(PlaceBidInput { listing_id, amount, user_id: bidder })
            .await
    }

    /// Add `listing_id` to `bidder`'s watchlist.
    pub async fn toggle_watch(
        &self,
        bidder: UserId,
        listing_id: ListingId,
    ) -> Result<(), DomainError> {
        self.reducer
            .watch_listing(WatchListingInput { listing_id, user_id: bidder })
            .await
            .map(|_| ())
    }

    /// List every bid placed by `bidder`.
    pub async fn list_my_bids(&self, bidder: UserId) -> Vec<Bid> {
        self.bid_repo.get_bids_by_user_id(bidder).await
    }
}

// docs/specs/ebay-spec-012, ebay-spec-013, ebay-spec-014, ebay-spec-015, ebay-spec-018, ebay-spec-021