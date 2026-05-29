// src/adapters/primary/http_axum/mod.rs

use adapters::secondary::image_store_fs::{ImageStoreFs, new as image_store_new, store_image, retrieve_image};
use adapters::secondary::password_hasher_argon2::{PasswordHasherArgon2, new as password_hasher_new, hash_password};
use adapters::secondary::stdb_client::{StdBClient, new as stdb_client_new, execute_query, analyze_data};
use core::ports::auction_repo::AuctionRepo;
use core::ports::listing_repo::{ListingRepo, ListingRepoImpl, Listing};
use core::ports::reducer_call::{ReducerCall, new as reducer_call_new, handle_bids};
use core::usecases::bidding::{BiddingUsecase, new as bidding_usecase_new, fetch_bids};

// ADR-2026-05-19-0721: Implement HTTP handlers using Axum

pub async fn store_image_handler(image_data: Vec<u8>) -> Result<String, String> {
    let image_store = ImageStoreFs::new();
    match store_image(&image_store, image_data) {
        Ok(image_id) => Ok(image_id),
        Err(e) => Err(format!("Failed to store image: {}", e)),
    }
}

pub async fn retrieve_image_handler(image_id: &str) -> Result<Vec<u8>, String> {
    let image_store = ImageStoreFs::new();
    match retrieve_image(&image_store, image_id) {
        Ok(image_data) => Ok(image_data),
        Err(e) => Err(format!("Failed to retrieve image: {}", e)),
    }
}

pub async fn hash_password_handler(password: &str) -> Result<String, String> {
    let password_hasher = PasswordHasherArgon2::new();
    match hash_password(&password_hasher, password) {
        Ok(hash) => Ok(hash),
        Err(e) => Err(format!("Failed to hash password: {}", e)),
    }
}

pub async fn execute_query_handler(query: &str) -> Result<String, String> {
    let stdb_client = StdBClient::new();
    match execute_query(&stdb_client, query) {
        Ok(result) => Ok(result),
        Err(e) => Err(format!("Failed to execute query: {}", e)),
    }
}

pub async fn analyze_data_handler(data: &str) -> Result<String, String> {
    let stdb_client = StdBClient::new();
    match analyze_data(&stdb_client, data) {
        Ok(result) => Ok(result),
        Err(e) => Err(format!("Failed to analyze data: {}", e)),
    }
}

pub async fn fetch_bids_handler(auction_id: &str) -> Result<Vec<String>, String> {
    let bidding_usecase = BiddingUsecase::new();
    match fetch_bids(&bidding_usecase, auction_id) {
        Ok(bids) => Ok(bids),
        Err(e) => Err(format!("Failed to fetch bids: {}", e)),
    }
}