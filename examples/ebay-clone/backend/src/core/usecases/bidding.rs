use crate::core::ports::auction_repo::{AuctionRepoPort};
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
        self.bid_repo.list_bids_by_user(user_id)
    }

    pub fn list_bids_for_listing(&self, listing_id: ListingId) -> Result<Vec<Bid>, DomainError> {
        self.bid_repo.list_bids_for_listing(listing_id)
    }

    pub fn list_active_auctions(&self) -> Result<Vec<Auction>, DomainError> {
        let auctions = self.auction_repo.fetch_auctions()?;
        Ok(auctions.into_iter().filter(|a| a.is_active()).collect())
    }

    pub fn toggle_watch(&self, input: WatchListingInput) -> Result<(), DomainError> {
        self.reducer_port.watch_listing(input)
    }
}

// ADR-2026-05-19-0721