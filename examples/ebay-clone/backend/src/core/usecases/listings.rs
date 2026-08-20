use crate::core::ports::listing_repo::SearchListingsParams;
use crate::core::ports::reducer_call::CreateListingInput;
use crate::core::ports::{ListingRepoPort, ReducerCallPort};
use crate::core::domain::{DomainError, Listing, UserId};

/// Use case for creating and querying listings.
///
/// Rewritten to conform to the port contracts: `ReducerCallPort::create_listing`
/// takes the owned `reducer_call::CreateListingInput` and returns
/// `Result<Listing, DomainError>`; `ListingRepoPort` is read-only and keyed on
/// `ListingId` / `SearchListingsParams`. The domain `Listing` has no `auction`
/// field, so the previous `listing.auction.*` access has been removed.
pub struct ListingsUsecase {
    reducer_port: Box<dyn ReducerCallPort>,
    repo_port: Box<dyn ListingRepoPort>,
}

impl ListingsUsecase {
    pub fn new(
        reducer_port: Box<dyn ReducerCallPort>,
        repo_port: Box<dyn ListingRepoPort>,
    ) -> Self {
        ListingsUsecase {
            reducer_port,
            repo_port,
        }
    }

    pub async fn create_listing(
        &self,
        user_id: UserId,
        title: String,
        description: String,
        starting_price_cents: u64,
        end_time: u64,
    ) -> Result<Listing, DomainError> {
        let input = CreateListingInput {
            title,
            description,
            starting_price: starting_price_cents,
            end_time,
            user_id,
        };
        self.reducer_port.create_listing(input).await
    }

    pub async fn search_listings(
        &self,
        query: Option<String>,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<Vec<Listing>, String> {
        let params = SearchListingsParams {
            query: query.map(|q| q.to_lowercase()),
            limit,
            offset,
        };
        self.repo_port
            .get_listings_by_criteria(&params)
            .await
            .map_err(|e| format!("Failed to search listings: {}", e))
    }

    pub async fn get_listing(
        &self,
        listing_id: crate::core::domain::ListingId,
    ) -> Result<Listing, String> {
        self.repo_port
            .get_listing_by_id(&listing_id)
            .await
            .map_err(|e| format!("Failed to get listing: {}", e))
    }
}

// Ref: docs/specs/ebay-spec-006
// Ref: docs/specs/ebay-spec-007
// Ref: docs/specs/ebay-spec-008
// Ref: docs/specs/ebay-spec-009
// Ref: docs/specs/ebay-spec-019