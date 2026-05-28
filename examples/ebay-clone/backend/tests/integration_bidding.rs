use std::net::TcpListener;
use actix_web::{web, App, HttpServer};
use spacetimedb_lib::reducer_call_port::ReducerCallPort;
use examples_ebay_clone_backend::app_state::AppState;
use examples_ebay_clone_backend::marketplace_module::MarketplaceModule;

#[actix_rt::test]
async fn test_bidding_success() {
    // ADR-2026-05-19-0721
    let stdb = spacetimedb_lib::test_harness::TestHarness::new().await;
    let module = MarketplaceModule::publish(&stdb).await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    stdb.register_adapter(&module, &server_url).await.unwrap();

    let app_state = AppState::new(stdb.clone());
    let server_handle = actix_web::rt::spawn(async move {
        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(app_state.clone()))
                .configure(examples_ebay_clone_backend::config_routes)
        })
        .listen(listener)
        .unwrap()
        .run()
        .await
    });

    let client = reqwest::Client::new();

    // Create a listing
    let create_listing_response = client.post(&format!("{}/listings", server_url))
        .json(&serde_json::json!({
            "title": "Test Item",
            "description": "A test item for bidding.",
            "start_price": 10.0,
            "end_time": "2030-01-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create_listing_response.status(), 201);
    let listing_id: u64 = create_listing_response.json().await.unwrap()["id"];

    // Place a bid on the listing
    let place_bid_response = client.post(&format!("{}/listings/{}/bids", server_url, listing_id))
        .json(&serde_json::json!({
            "amount": 15.0
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(place_bid_response.status(), 201);
    let bid: serde_json::Value = place_bid_response.json().await.unwrap();
    assert_eq!(bid["amount"], 15.0);

    server_handle.abort();
}

#[actix_rt::test]
async fn test_bidding_insufficient_funds() {
    // ADR-2026-05-19-0721
    let stdb = spacetimedb_lib::test_harness::TestHarness::new().await;
    let module = MarketplaceModule::publish(&stdb).await.unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", port);

    stdb.register_adapter(&module, &server_url).await.unwrap();

    let app_state = AppState::new(stdb.clone());
    let server_handle = actix_web::rt::spawn(async move {
        HttpServer::new(move || {
            App::new()
                .app_data(web::Data::new(app_state.clone()))
                .configure(examples_ebay_clone_backend::config_routes)
        })
        .listen(listener)
        .unwrap()
        .run()
        .await
    });

    let client = reqwest::Client::new();

    // Create a listing with a high start price
    let create_listing_response = client.post(&format!("{}/listings", server_url))
        .json(&serde_json::json!({
            "title": "High Priced Item",
            "description": "An item with a very high starting price.",
            "start_price": 10000.0,
            "end_time": "2030-01-01T00:00:00Z"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(create_listing_response.status(), 201);
    let listing_id: u64 = create_listing_response.json().await.unwrap()["id"];

    // Attempt to place a low bid on the listing
    let place_bid_response = client.post(&format!("{}/listings/{}/bids", server_url, listing_id))
        .json(&serde_json::json!({
            "amount": 5.0
        }))
        .send()
        .await