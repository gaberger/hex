use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json, TypedHeader,
};
use axum_extra::extract::cookie::CookieJar;
use jsonwebtoken::{decode, DecodingKey, Validation};
use serde_json::json;

use crate::adapters::primary::http_axum::AppState;
use crate::application::{
    use_cases::{self, GetBidsForUser, GetWonAuctionsForUser, GetUserListings},
    domain::UserIdentity,
};

// ADR-2026-05-19-0721

async fn authenticate_user(jar: CookieJar) -> Result<UserIdentity, StatusCode> {
    let token = jar.get("auth_token").ok_or(StatusCode::UNAUTHORIZED)?;
    let secret_key = "your_secret_key"; // Replace with a secure method to retrieve the key
    let decoding_key = DecodingKey::from_secret(secret_key.as_ref());
    let validation = Validation::default();
    let claims = decode::<UserIdentity>(token.value(), &decoding_key, &validation)
        .map_err(|_| StatusCode::UNAUTHORIZED)?;
    Ok(claims.claims)
}

pub async fn get_my_bids(
    jar: CookieJar,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let user_identity = authenticate_user(jar).await?;
    let bids = use_cases::get_bids_for_user(&state.repo, &user_identity).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::OK, Json(json!(bids))))
}

pub async fn get_my_won_items(
    jar: CookieJar,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let user_identity = authenticate_user(jar).await?;
    let won_auctions = use_cases::get_won_auctions_for_user(&state.repo, &user_identity)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::OK, Json(json!(won_auctions))))
}

pub async fn get_my_listings(
    jar: CookieJar,
    State(state): State<AppState>,
) -> Result<(StatusCode, Json<serde_json::Value>), StatusCode> {
    let user_identity = authenticate_user(jar).await?;
    let listings = use_cases::get_user_listings(&state.repo, &user_identity)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok((StatusCode::OK, Json(json!(listings))))
}