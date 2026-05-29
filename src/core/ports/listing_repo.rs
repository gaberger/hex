// hex-core/src/core/ports/listing_repo.rs

use adapters::secondary::stdb_client::StdBClient;
use core::usecases::bidding::BiddingUsecase;

pub trait ListingRepo {
    fn fetch_listings(&self) -> Vec<Listing>;
}

pub struct ListingRepoImpl {
    client: StdBClient,
}

impl ListingRepoImpl {
    pub fn new(client: StdBClient) -> Self {
        ListingRepoImpl { client }
    }
}

impl ListingRepo for ListingRepoImpl {
    fn fetch_listings(&self) -> Vec<Listing> {
        let query = "SELECT * FROM listings";
        let data = self.client.execute_query(query).unwrap();
        data.into_iter().map(|row| Listing::from(row)).collect()
    }
}

#[derive(Debug)]
pub struct Listing {
    pub id: u32,
    pub title: String,
    pub price: f64,
}

impl From<Vec<String>> for Listing {
    fn from(row: Vec<String>) -> Self {
        Listing {
            id: row[0].parse().unwrap(),
            title: row[1].clone(),
            price: row[2].parse().unwrap(),
        }
    }
}