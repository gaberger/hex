use super::{ReducerCallPort, ListingRepoPort};
use crate::core::entities::{Listing, Auction};
use crate::core::domain_types::{Title, PriceCents, Duration};

pub struct ListingsUsecase {
    reducer_port: Box<dyn ReducerCallPort>,
    repo_port: Box<dyn ListingRepoPort>,
}

impl ListingsUsecase {
    pub fn new(reducer_port: Box<dyn ReducerCallPort>, repo_port: Box<dyn ListingRepoPort>) -> Self {
        ListingsUsecase { reducer_port, repo_port }
    }

    pub fn create_listing(&self, title: String, price_cents: u32, duration: u64, image_sha256s: Vec<String>) -> Result<Listing, String> {
        let title = Title::new(title).map_err(|_| "Invalid title")?;
        let price_cents = PriceCents::new(price_cents).map_err(|_| "Invalid price")?;
        let duration = Duration::new(duration).map_err(|_| "Invalid duration")?;

        self.reducer_port.create_listing(&title, &price_cents, &duration, image_sha256s)
            .and_then(|listing_id| self.repo_port.get_listing(listing_id))
            .map_err(|e| format!("Failed to create listing: {}", e))
    }

    pub fn search_listings(&self, q: String, active: bool, max_price_cents: Option<u32>, page: usize, per_page: usize) -> Result<Vec<Listing>, String> {
        let listings = self.repo_port.search_listings(q.to_lowercase(), active, max_price_cents.map(PriceCents::new))
            .map_err(|e| format!("Failed to search listings: {}", e))?;

        // Sort by end_time ASC
        let mut sorted_listings = listings.into_iter().filter(|listing| listing.auction.active).collect::<Vec<Listing>>();
        sorted_listings.sort_by_key(|listing| listing.auction.end_time);

        // Paginate
        let start = page * per_page;
        let end = start + per_page;
        Ok(sorted_listings[start..end.min(sorted_listings.len())].to_vec())
    }

    pub fn get_listing(&self, listing_id: String) -> Result<(Listing, Auction), String> {
        self.repo_port.get_listing(listing_id)
            .and_then(|listing| {
                let auction = listing.auction.clone();
                Ok((listing, auction))
            })
            .map_err(|e| format!("Failed to get listing: {}", e))
    }
}

// Ref: docs/specs/ebay-spec-006
// Ref: docs/specs/ebay-spec-007
// Ref: docs/specs/ebay-spec-008
// Ref: docs/specs/ebay-spec-009
// Ref: docs/specs/ebay-spec-019