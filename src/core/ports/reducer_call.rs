// hex-core/src/core/ports/reducer_call.rs

use core::ports::AuctionRepo;
use core::ports::ListingRepo;
use core::usecases::bidding::BiddingUsecase;

pub struct ReducerCall {
    auction_repo: AuctionRepoImpl,
    listing_repo: ListingRepoImpl,
    bidding_usecase: BiddingUsecase,
}

impl ReducerCall {
    pub fn new(auction_repo: AuctionRepoImpl, listing_repo: ListingRepoImpl) -> Self {
        let bidding_usecase = BiddingUsecase::new(&auction_repo);
        ReducerCall {
            auction_repo,
            listing_repo,
            bidding_usecase,
        }
    }

    pub fn handle_bids(&self) {
        self.bidding_usecase.fetch_bids();
    }
}