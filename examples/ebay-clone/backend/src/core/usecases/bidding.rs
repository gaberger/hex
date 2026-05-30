use crate::core::ports::auction_repo::{AuctionRepoPort};
use crate::core::ports::bid_repo::BidRepoPort;
use crate::core::ports::reducer_call::{ReducerCallPort, PlaceBidInput, WatchListingInput};
use crate::core::domain::{Bid, DomainError, ListingId, UserId, WatchEntry, Auction};

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
        BiddingUseCase {
            bid_repo,
            auction_repo,
            reducer_port,
        }
    }

    pub fn place_bid(&self, input: PlaceBidInput) -> Result<(), DomainError> {
        self.reducer_port.place_bid(input)
    }

    pub fn list_my_bids(&self, user_id: UserId) -> Result<Vec<Bid>, DomainError> {
        self.bid_repo.get_bids_by_user(user_id)
    }

    pub fn list_bids_for_listing(&self, listing_id: ListingId) -> Result<Vec<Bid>, DomainError> {
        self.bid_repo.get_bids_by_listing(listing_id)
    }

    pub fn list_active_auctions(&self) -> Result<Vec<Auction>, DomainError> {
        self.auction_repo.get_active_auctions()
    }

    pub fn toggle_watch(&self, user_id: UserId, listing_id: ListingId) -> Result<(), DomainError> {
        self.reducer_port.toggle_watch(WatchListingInput { user_id, listing_id })
    }
}

ADR-2026-05-19-0721