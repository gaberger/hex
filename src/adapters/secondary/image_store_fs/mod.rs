// ADR-2026-05-19-0721

use core::ports::listing_repo::{ListingRepo, Listing};
use adapters::secondary::stdb_client::{StdBClient, new as stdb_new};

pub struct ImageStoreFs {
    client: StdBClient,
}

impl ImageStoreFs {
    pub fn new(client: StdBClient) -> Self {
        ImageStoreFs { client }
    }

    pub fn store_image(&self, listing_id: &str, image_data: &[u8]) -> Result<(), String> {
        // Placeholder implementation
        Ok(())
    }

    pub fn retrieve_image(&self, listing_id: &str) -> Result<Vec<u8>, String> {
        // Placeholder implementation
        Ok(vec![])
    }
}