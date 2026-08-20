use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;
use tower_cookies::Cookies;

use crate::adapters::primary::http_axum::AppState;
use crate::application::errors::AppError;
use crate::domain::auctions::BidPlacementRequest;
use crate::usecases::place_bid_use_case::{self, PlaceBidUseCase};

// ADR-2026-05-19-0721

/// Handles the POST /api/v1/listings/:id/bid request.
///
/// # Arguments
///
/// * `cookies` - The cookies to extract the JWT token from.
/// * `state` - The state of the application.
/// * `Path(listing_id)` - The ID of the listing to bid on.
/// * `Json(payload)` - The payload containing the bid amount.
///
/// # Returns
///
/// * `201 Created` if the bid was successfully placed.
/// * `409 Conflict` if the bid is too low.
/// * `410 Gone` if the auction has ended.
/// * `403 Forbidden` if the user tries to bid on their own listing.
/// * `500 Internal Server Error` for other errors.
pub async fn place_bid(
    cookies: Cookies,
    State(state): State<AppState>,
    Path(listing_id): Path<String>,
    Json(payload): Json<BidPlacementRequest>,
) -> impl IntoResponse {
    let token = match cookies.get("auth_token") {
        Some(cookie) => cookie.value().to_string(),
        None => return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Unauthorized" }))),
    };

    let use_case = PlaceBidUseCase::new(state.repo.clone(), state.authenticator.clone());

    match use_case.execute(token, listing_id, payload).await {
        Ok(_) => StatusCode::CREATED.into_response(),
        Err(AppError::BidTooLow) => (StatusCode::CONFLICT, Json(json!({ "error": "bid_too_low" }))).into_response(),
        Err(AppError::AuctionEnded) => (StatusCode::GONE, Json(json!({ "error": "auction_ended" }))).into_response(),
        Err(AppError::SelfBidForbidden) => (StatusCode::FORBIDDEN, Json(json!({ "error": "self_bid_forbidden" }))).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Handles the POST /api/v1/listings/:id/watch request.
///
/// # Arguments
///
/// * `cookies` - The cookies to extract the JWT token from.
/// * `state` - The state of the application.
/// * `Path(listing_id)` - The ID of the listing to toggle watch status on.
///
/// # Returns
///
/// * `200 OK` if the watchlist was successfully toggled.
/// * `500 Internal Server Error` for other errors.
pub async fn toggle_watch(
    cookies: Cookies,
    State(state): State<AppState>,
    Path(listing_id): Path<String>,
) -> impl IntoResponse {
    let token = match cookies.get("auth_token") {
        Some(cookie) => cookie.value().to_string(),
        None => return (StatusCode::UNAUTHORIZED, Json(json!({ "error": "Unauthorized" }))),
    };

    // Placeholder for toggle_watch use case
    // Assuming toggle_watch_use_case is implemented similarly to place_bid_use_case

    match state.repo.toggle_watch(&token, &listing_id).await {
        Ok(_) => StatusCode::OK.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}