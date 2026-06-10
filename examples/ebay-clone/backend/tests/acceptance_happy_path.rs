//! In-process acceptance test for the eBay-clone backend happy path.
//!
//! Drives the REAL axum router (`composition_root::compose_app`) end-to-end over
//! HTTP via `tower::ServiceExt::oneshot`: register two users, post a listing,
//! place a winning bid, let the auction close, and assert the winner — plus the
//! key negative paths (seller self-bid, bid after close). No browser, no socket,
//! no SpacetimeDB; the in-memory marketplace adapter stands in for STDB, so this
//! exercises the hex-built domain + ports + use cases + adapters as a working
//! system.
//!
//! This replaces the original persona-authored `fantoccini` browser test, which
//! was not valid Rust (a stray ```` ```rust ```` fence on line 1), depended on
//! crates absent from the manifest, and required a full live stack to run.
//!
//! Run: `cargo test --test acceptance_happy_path`

use std::time::Duration;

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use ebay_clone_backend::composition_root::compose_app;
use serde_json::{json, Value};
use tower::ServiceExt; // brings `oneshot` into scope

async fn send(
    app: &Router,
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(t) = token {
        builder = builder.header("authorization", format!("Bearer {t}"));
    }
    let req = builder.body(Body::from(body.to_string())).unwrap();

    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let val = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    };
    (status, val)
}

#[tokio::test]
async fn happy_path_register_post_bid_close_win() {
    let app = compose_app();

    // 1. Register userA (seller) and userB (bidder).
    let (s, a) = send(
        &app,
        "POST",
        "/api/v1/auth/register",
        None,
        json!({ "username": "userA", "email": "a@example.com", "password": "password123" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "register userA failed: {a}");
    let token_a = a["token"].as_str().expect("token A").to_string();

    let (s, b) = send(
        &app,
        "POST",
        "/api/v1/auth/register",
        None,
        json!({ "username": "userB", "email": "b@example.com", "password": "password123" }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "register userB failed: {b}");
    let token_b = b["token"].as_str().expect("token B").to_string();

    // 2. userA posts a listing with a 1-second auction.
    let (s, listing) = send(
        &app,
        "POST",
        "/api/v1/listings",
        Some(&token_a),
        json!({
            "title": "Vintage Camera",
            "description": "mint condition",
            "starting_price_cents": 1000,
            "duration_secs": 1
        }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "create listing failed: {listing}");
    let listing_id = listing["listing_id"].as_u64().expect("listing_id");
    assert!(listing_id > 0, "listing_id should be assigned");

    // Creating a listing requires auth.
    let (s, _) = send(
        &app,
        "POST",
        "/api/v1/listings",
        None,
        json!({ "title": "x", "starting_price_cents": 1, "duration_secs": 1 }),
    )
    .await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "unauthenticated listing must 401");

    // 3. userB places a winning bid above the starting price.
    let (s, bid) = send(
        &app,
        "POST",
        &format!("/api/v1/listings/{listing_id}/bids"),
        Some(&token_b),
        json!({ "amount_cents": 1500 }),
    )
    .await;
    assert_eq!(s, StatusCode::CREATED, "place bid failed: {bid}");

    // Negative: the seller cannot bid on their own auction.
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/v1/listings/{listing_id}/bids"),
        Some(&token_a),
        json!({ "amount_cents": 2000 }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "seller self-bid must be rejected");

    // Negative: a bid at or below the current high bid is rejected.
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/v1/listings/{listing_id}/bids"),
        Some(&token_b),
        json!({ "amount_cents": 1500 }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "non-increasing bid must be rejected");

    // 4. Let the auction close.
    tokio::time::sleep(Duration::from_millis(1500)).await;

    // Negative: bids after close are rejected.
    let (s, _) = send(
        &app,
        "POST",
        &format!("/api/v1/listings/{listing_id}/bids"),
        Some(&token_b),
        json!({ "amount_cents": 5000 }),
    )
    .await;
    assert_eq!(s, StatusCode::CONFLICT, "bid after close must be rejected");

    // 5. userB sees the won item; userA (seller) wins nothing.
    let (s, won_b) = send(&app, "GET", "/api/v1/me/won", Some(&token_b), Value::Null).await;
    assert_eq!(s, StatusCode::OK, "my-won failed: {won_b}");
    let items = won_b["won"].as_array().expect("won array");
    assert_eq!(items.len(), 1, "userB should have one won item: {won_b}");
    assert_eq!(items[0]["winner"].as_str().unwrap(), "userB");
    assert_eq!(items[0]["listing_id"].as_u64().unwrap(), listing_id);
    assert_eq!(items[0]["amount_cents"].as_u64().unwrap(), 1500);

    let (s, won_a) = send(&app, "GET", "/api/v1/me/won", Some(&token_a), Value::Null).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        won_a["won"].as_array().unwrap().len(),
        0,
        "seller should win nothing"
    );
}
