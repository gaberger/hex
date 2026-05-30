use crate::core::ports::auction_repo::{AuctionRepoPort, Auction};
use crate::core::ports::bid_repo::BidRepoPort;
use crate::core::ports::reducer_call::{ReducerCallPort, PlaceBidInput, WatchListingInput};
use crate::core::domain::{Bid, DomainError, ListingId, UserId, WatchEntry};

/// Use case for handling bidding actions.
///
/// Conforms to the port contracts: every method delegates to a port trait
/// method with the exact signature declared in `core::ports`.
pub struct BiddingUseCase {
    bid_repo: Box<dyn BidRepoPort>,
    auction_repo: Box<dyn AuctionRepoPort>,
    reducer_port: Box<dyn ReducerCallPort>,
}

impl BiddingUseCase {
    pub fn new(
        bid_repo: Box<dyn BidRepoPort>,
        auction_repo: Box<dyn AuctionRepoPort>,
        reducer_port: Box<dyn ReducerCallPort>,
    ) -> Self {
        BiddingUseCase { bid_repo, auction_repo, reducer_port }
    }

    /// Places a bid on an auction via the reducer port.
    pub async fn place_bid(&self, input: PlaceBidInput) -> Result<Bid, DomainError> {
        self.reducer_port.place_bid(input).await
    }

    /// Lists all bids made by the given bidder.
    pub async fn list_my_bids(&self, user_id: UserId) -> Vec<Bid> {
        self.bid_repo.get_bids_by_user_id(user_id).await
    }

    /// Lists all bids placed on a specific listing.
    pub async fn list_bids_for_listing(&self, listing_id: ListingId) -> Vec<Bid> {
        self.bid_repo.get_bids_by_listing_id(listing_id).await
    }

    /// Lists all auctions currently active.
    pub async fn list_active_auctions(&self) -> Result<Vec<Auction>, DomainError> {
        self.auction_repo.list_active_auctions().await
    }

    /// Toggles a listing in the user's watchlist via the reducer port.
    pub async fn toggle_watch(&self, input: WatchListingInput) -> Result<WatchEntry, DomainError> {
        self.reducer_port.watch_listing(input).await
    }
}

// ADR-2026-05-19-0721