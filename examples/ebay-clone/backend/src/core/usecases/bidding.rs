use crate::core::ports::auction_repo::Auction;
use crate::core::ports::{
    AuctionRepoPort, BidRepoPort, PlaceBidInput, ReducerCallPort, WatchListingInput,
};
use crate::core::domain::{Bid, DomainError, ListingId, UserId, WatchEntry};

/// Use case for handling bidding actions.
///
/// Conforms to the port contracts exactly: every repository/reducer call is
/// async and uses only methods that the ports actually declare. The original
/// draft invoked `find_by_bidder` / `find_by_winner` / `find_by_seller` and
/// `DomainError::{Unexpected,RepoAccessFailed}`, none of which exist in the
/// inner layers — those calls have been replaced with the real port surface.
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
        BiddingUseCase {
            bid_repo,
            auction_repo,
            reducer_port,
        }
    }

    /// Places a bid on an auction via the reducer port.
    pub async fn place_bid(&self, input: PlaceBidInput) -> Result<Bid, DomainError> {
        self.reducer_port.place_bid(input).await
    }

    /// Lists all bids made by the given bidder.
    pub async fn list_my_bids(&self, bidder_id: UserId) -> Vec<Bid> {
        self.bid_repo.get_bids_by_user_id(bidder_id).await
    }

    /// Lists all bids placed on a given listing.
    pub async fn list_bids_for_listing(&self, listing_id: ListingId) -> Vec<Bid> {
        self.bid_repo.get_bids_by_listing_id(listing_id).await
    }

    /// Toggles a listing in the user's watchlist via the reducer port.
    pub async fn toggle_watch(
        &self,
        input: WatchListingInput,
    ) -> Result<WatchEntry, DomainError> {
        self.reducer_port.watch_listing(input).await
    }

    /// Lists all currently active auctions.
    pub async fn list_active_auctions(&self) -> Result<Vec<Auction>, DomainError> {
        self.auction_repo.list_active_auctions().await
    }
}

// ADR-2026-05-19-0721