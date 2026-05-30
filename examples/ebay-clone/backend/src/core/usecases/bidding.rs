use super::ports::{BidRepoPort, AuctionRepoPort, ReducerCallPort};
use crate::core::domain::{Listing, DomainError};

/// Use case for handling bidding actions.
pub struct BiddingUseCase {
    bid_repo: Box<dyn BidRepoPort>,
    auction_repo: Box<dyn AuctionRepoPort>,
    reducer_port: Box<dyn ReducerCallPort>,
}

impl BiddingUseCase {
    pub fn new(bid_repo: Box<dyn BidRepoPort>, auction_repo: Box<dyn AuctionRepoPort>, reducer_port: Box<dyn ReducerCallPort>) -> Self {
        BiddingUseCase { bid_repo, auction_repo, reducer_port }
    }

    /// Places a bid on an auction.
    pub fn place_bid(&self, bid: crate::core::domain::Bid) -> Result<(), DomainError> {
        self.reducer_port.place_bid(bid)
            .map_err(|_| DomainError::Unexpected)
    }

    /// Lists all bids made by the given bidder.
    pub fn list_my_bids(&self, bidder_id: UserId) -> Result<Vec<Bid>, DomainError> {
        self.bid_repo.get_bids_by_user(bidder_id)
            .map_err(|_| DomainError::Unexpected)
    }

    /// Lists all auctions won by the given bidder.
    pub fn list_my_won_auctions(&self, bidder_id: UserId) -> Result<Vec<Auction>, DomainError> {
        self.auction_repo.get_won_auctions(bidder_id)
            .map_err(|_| DomainError::Unexpected)
    }

    /// Toggles watching an auction for the given bidder.
    pub fn toggle_watch_auction(&self, bidder_id: UserId, auction_id: AuctionId) -> Result<(), DomainError> {
        self.reducer_port.watch_listing(WatchListingInput { user_id: bidder_id, listing_id: auction_id })
            .map_err(|_| DomainError::Unexpected)
    }

    /// Lists all listings watched by the given bidder.
    pub fn list_my_listings(&self, bidder_id: UserId) -> Result<Vec<Listing>, DomainError> {
        self.auction_repo.get_watched_auctions(bidder_id)
            .map_err(|_| DomainError::Unexpected)
    }
}

// ADR-2026-05-19-0721