use crate::core::ports::{ReducerCallPort, ListingRepoPort};
use crate::core::ports::reducer_call::CreateListingInput;
use crate::core::ports::listing_repo::SearchListingsParams;
use crate::core::domain::{Listing, ListingId};

pub struct ListingsUsecase {
    reducer_port: Box<dyn ReducerCallPort>,
    repo_port: Box<dyn ListingRepoPort>,
}

impl ListingsUsecase {
    pub fn new(reducer_port: Box<dyn ReducerCallPort>, repo_port: Box<dyn ListingRepoPort>) -> Self {
        ListingsUsecase { reducer_port, repo_port }
    }

    pub async fn create_listing(&self, input: CreateListingInput) -> Result<Listing, String> {
        self.reducer_port
            .create_listing(input)
            .await
            .map_err(|e| format!("Failed to create listing: {}", e))
    }

    pub async fn search_listings(&self, criteria: SearchListingsParams) -> Result<Vec<Listing>, String> {
        self.repo_port
            .get_listings_by_criteria(&criteria)
            .await
            .map_err(|e| format!("Failed to search listings: {:?}", e))
    }

    pub async fn get_listing(&self, listing_id: ListingId) -> Result<Listing, String> {
        self.repo_port
            .get_listing_by_id(&listing_id)
            .await
            .map_err(|e| format!("Failed to get listing: {:?}", e))
    }
}

// Ref: docs/specs/ebay-spec-006
// Ref: docs/specs/ebay-spec-007
// Ref: docs/specs/ebay-spec-008
// Ref: docs/specs/ebay-spec-009
// Ref: docs/specs/ebay-spec-019