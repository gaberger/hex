use crate::core::ports::{BidRepoPort, AuctionRepoPort, ReducerCallPort};
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

    /// Lists my bids.
    pub fn list_my_bids(&self, user_id: &crate::core::domain::UserId) -> Result<Vec<crate::core::domain::Bid>, DomainError> {
        self.bid_repo.get_user_bids(user_id)
            .map_err(|_| DomainError::Unexpected)
    }

    /// Lists auctions I have won.
    pub fn list_my_won(&self, user_id: &crate::core::domain::UserId) -> Result<Vec<crate::core::domain::Auction>, DomainError> {
        self.auction_repo.get_user_won_auctions(user_id)
            .map_err(|_| DomainError::Unexpected)
    }

    /// Toggles watching an auction.
    pub fn toggle_watch(&self, user_id: &crate::core::domain::UserId, auction_id: &crate::core::domain::AuctionId) -> Result<(), DomainError> {
        self.reducer_port.watch_listing(crate::core::ports::WatchListingInput {
            user_id: user_id.clone(),
            auction_id: auction_id.clone(),
        })
        .map_err(|_| DomainError::Unexpected)
    }

    /// Lists my listings.
    pub fn list_my_listings(&self, user_id: &crate::core::domain::UserId) -> Result<Vec<crate::core::domain::Listing>, DomainError> {
        self.auction_repo.get_user_listings(user_id)
            .map_err(|_| DomainError::Unexpected)
    }
}

ADR-2026-05-19-0721