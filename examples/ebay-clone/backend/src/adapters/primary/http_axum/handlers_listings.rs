use axum::{
    extract::{Extension, Path, Query},
    http::StatusCode,
    Json, Router,
};
use serde::{Deserialize, Serialize};
use tower_http::auth::RequireAuthorizationLayer;
use uuid::Uuid;

use super::super::ports::{CreateListingUseCasePort, ListingRepoPort};

#[derive(Serialize)]
pub struct ListingResponse {
    id: Uuid,
    // other fields as per your domain model
}

#[derive(Deserialize)]
pub struct CreateListingRequest {
    // fields for creating a listing
}

#[derive(Deserialize)]
pub struct SearchListingsParams {
    q: Option<String>,
    active: Option<bool>,
    max_price_cents: Option<u32>,
    limit: Option<usize>,
    offset: Option<usize>,
}

impl Default for SearchListingsParams {
    fn default() -> Self {
        Self {
            q: None,
            active: Some(true),
            max_price_cents: None,
            limit: Some(20),
            offset: Some(0),
        }
    }
}

pub async fn create_listing(
    Extension(create_listing_use_case): Extension<Box<dyn CreateListingUseCasePort>>,
    Json(request): Json<CreateListingRequest>,
) -> Result<(StatusCode, Json<ListingResponse>), StatusCode> {
    // Implement auth check here
    let listing = create_listing_use_case.create_listing(request).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((
        StatusCode::CREATED,
        Json(ListingResponse { id: listing.id /* other fields */ }),
    ))
}

pub async fn get_listings(
    Extension(listing_repo): Extension<Box<dyn ListingRepoPort>>,
    Query(params): Query<SearchListingsParams>,
) -> Result<Json<Vec<ListingResponse>>, StatusCode> {
    let listings = listing_repo
        .search_listings(
            params.q.as_deref(),
            params.active,
            params.max_price_cents,
            params.limit.unwrap_or(20).clamp(1, 100),
            params.offset.unwrap_or(0),
        )
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(
        listings.into_iter().map(|l| ListingResponse { id: l.id /* other fields */ }).collect(),
    ))
}

pub async fn get_listing_by_id(
    Extension(listing_repo): Extension<Box<dyn ListingRepoPort>>,
    Path(id): Path<Uuid>,
) -> Result<Json<ListingResponse>, StatusCode> {
    let listing = listing_repo
        .find_listing_with_auction(id)
        .await
        .map_err(|_| StatusCode::NOT_FOUND)?;
    Ok(Json(ListingResponse { id: listing.id /* other fields */ }))
}

pub fn listings_routes() -> Router {
    Router::new()
        .route("/api/v1/listings", axum::routing::post(create_listing))
        .route("/api/v1/listings", axum::routing::get(get_listings))
        .route("/api/v1/listings/:id", axum::routing::get(get_listing_by_id))
        .layer(RequireAuthorizationLayer::bearer("secret_token"))
}

// ADR-2026-05-19-0721
// docs/specs/ebay-spec-006, ebay-spec-007, ebay-spec-008, ebay-spec-009, ebay-spec-019