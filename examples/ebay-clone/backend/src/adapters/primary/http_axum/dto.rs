use serde::{Deserialize, Serialize};

// ADR-2026-05-19-0721

#[derive(Debug, Serialize, Deserialize)]
pub struct UserRequest {
    pub username: String,
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserResponse {
    pub id: u32,
    pub username: String,
    pub email: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ItemRequest {
    pub title: String,
    pub description: String,
    pub starting_bid: f64,
    pub end_time: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ItemResponse {
    pub id: u32,
    pub title: String,
    pub description: String,
    pub starting_bid: f64,
    pub current_bid: Option<f64>,
    pub end_time: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BidRequest {
    pub item_id: u32,
    pub bid_amount: f64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct BidResponse {
    pub id: u32,
    pub user_id: u32,
    pub item_id: u32,
    pub bid_amount: f64,
}