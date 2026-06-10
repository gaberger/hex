//! Coherent axum HTTP surface for the eBay-clone backend.
//!
//! A single [`AppState`] (holding the three application use cases plus the
//! auction read port) is shared across all handlers via axum's `State`
//! extractor — replacing the three mutually-incompatible `AppState`
//! definitions and DI conventions (`Extension` vs `State` vs concrete state)
//! that the per-agent handler files were each written against in isolation.
//!
//! Identity: register issues a bearer token; protected endpoints resolve it
//! back to a [`UserId`] via [`TokenIssuerPort`] (`AuthUseCase::verify_token`).

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use tower_http::cors::{Any, CorsLayer};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::core::domain::{ListingId, Timestamp, UserId};
use crate::core::ports::auction_repo::AuctionRepoPort;
use crate::core::ports::reducer_call::PlaceBidInput;
use crate::core::usecases::auth::AuthUseCase;
use crate::core::usecases::bidding::BiddingUseCase;
use crate::core::usecases::listings::ListingsUsecase;

/// Shared application state. `Clone` is cheap — every field is an `Arc`.
#[derive(Clone)]
pub struct AppState {
    pub auth: Arc<AuthUseCase>,
    pub listings: Arc<ListingsUsecase>,
    pub bidding: Arc<BiddingUseCase>,
    pub auction_repo: Arc<dyn AuctionRepoPort>,
}

/// Builds the router with every marketplace route mounted on `state`.
pub fn build_router(state: AppState) -> Router {
    // Permissive CORS so the Solid frontend (a different origin, e.g.
    // localhost:5173) can call this API from the browser. Fine for an example;
    // a real deployment would pin `allow_origin` to the known frontend origin.
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);
    Router::new()
        .route("/api/v1/health", get(|| async { "OK" }))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/listings", post(create_listing).get(list_listings))
        .route("/api/v1/listings/:id/bids", post(place_bid))
        .route("/api/v1/me/won", get(my_won))
        .layer(cors)
        .with_state(state)
}

/// Resolves the bearer token in `Authorization` to a [`UserId`].
async fn authed_user(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<UserId, (StatusCode, String)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or((StatusCode::UNAUTHORIZED, "missing bearer token".to_string()))?;
    let user = state
        .auth
        .verify_token(token)
        .await
        .map_err(|e| (StatusCode::UNAUTHORIZED, e.to_string()))?;
    Ok(UserId(user.canonical_username))
}

#[derive(Deserialize)]
struct RegisterReq {
    username: String,
    #[serde(default)]
    email: Option<String>,
    password: String,
}

async fn register(State(state): State<AppState>, Json(req): Json<RegisterReq>) -> impl IntoResponse {
    let user = match state
        .auth
        .register_user(req.username, req.email.unwrap_or_default(), req.password)
        .await
    {
        Ok(u) => u,
        Err(e) => return (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    };
    match state.auth.issue_token(&user).await {
        Ok(token) => (
            StatusCode::CREATED,
            Json(json!({ "token": token, "username": user.canonical_username })),
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Deserialize)]
struct CreateListingReq {
    title: String,
    #[serde(default)]
    description: Option<String>,
    starting_price_cents: u64,
    duration_secs: u64,
}

async fn create_listing(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<CreateListingReq>,
) -> impl IntoResponse {
    let user = match authed_user(&state, &headers).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };
    let end_time = Utc::now().timestamp() as u64 + req.duration_secs.max(1);
    match state
        .listings
        .create_listing(
            user,
            req.title,
            req.description.unwrap_or_default(),
            req.starting_price_cents,
            end_time,
        )
        .await
    {
        Ok(listing) => (
            StatusCode::CREATED,
            Json(json!({
                "listing_id": listing.id.parse::<u64>().unwrap_or(0),
                "title": listing.title,
            })),
        )
            .into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn list_listings(State(state): State<AppState>) -> impl IntoResponse {
    match state.listings.search_listings(None, None, None).await {
        Ok(ls) => {
            let items: Vec<Value> = ls
                .iter()
                .map(|l| {
                    json!({
                        "listing_id": l.id.parse::<u64>().unwrap_or(0),
                        "title": l.title,
                        "starting_price_cents": l.starting_price_cents,
                    })
                })
                .collect();
            Json(json!({ "listings": items })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

#[derive(Deserialize)]
struct BidReq {
    amount_cents: u64,
}

async fn place_bid(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    headers: HeaderMap,
    Json(req): Json<BidReq>,
) -> impl IntoResponse {
    let user = match authed_user(&state, &headers).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };
    let input = PlaceBidInput {
        listing_id: ListingId(id),
        amount: req.amount_cents,
        user_id: user,
    };
    match state.bidding.place_bid(input).await {
        Ok(bid) => (
            StatusCode::CREATED,
            Json(json!({ "accepted": true, "amount_cents": bid.amount_cents, "bidder": bid.bidder_id })),
        )
            .into_response(),
        // Bid rejections (too low, ended, self-bid) are client conflicts.
        Err(e) => (StatusCode::CONFLICT, e.to_string()).into_response(),
    }
}

async fn my_won(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    let user = match authed_user(&state, &headers).await {
        Ok(u) => u,
        Err(e) => return e.into_response(),
    };
    let ended = match state
        .auction_repo
        .list_recently_ended_auctions(
            Timestamp::try_from(0).unwrap(),
            Timestamp::try_from(i64::MAX).unwrap(),
        )
        .await
    {
        Ok(a) => a,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let won: Vec<Value> = ended
        .into_iter()
        .filter_map(|a| {
            let cb = a.current_bid?;
            if cb.bidder_id == user {
                Some(json!({
                    "listing_id": a.listing_id.0,
                    "winner": cb.bidder_id.0,
                    "amount_cents": cb.amount.cents(),
                }))
            } else {
                None
            }
        })
        .collect();
    Json(json!({ "won": won })).into_response()
}
