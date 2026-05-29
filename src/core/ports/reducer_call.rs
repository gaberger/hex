// hex-core/src/core/ports/reducer_call.rs

use core::ports::AuctionRepo;
use core::ports::ListingRepo;
use core::usecases::bidding::BiddingUsecase;
use async_trait::async_trait;

#[async_trait]
pub trait ReducerCall {
    async fn new(auction_repo: AuctionRepo, listing_repo: ListingRepo) -> Self;
    async fn handle_bids(&self);
}

pub struct ReducerCallImpl {
    auction_repo: AuctionRepo,
    listing_repo: ListingRepo,
    bidding_usecase: BiddingUsecase,
}

#[async_trait]
impl ReducerCall for ReducerCallImpl {
    async fn new(auction_repo: AuctionRepo, listing_repo: ListingRepo) -> Self {
        let bidding_usecase = BiddingUsecase::new(&auction_repo);
        ReducerCallImpl {
            auction_repo,
            listing_repo,
            bidding_usecase,
        }
    }

    async fn handle_bids(&self) {
        self.bidding_usecase.fetch_bids().await;
    }
}