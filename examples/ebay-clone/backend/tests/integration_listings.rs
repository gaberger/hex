// examples/ebay-clone/backend/tests/integration_listings.rs

use std::net::{SocketAddr, TcpListener};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use reqwest::Client;
use spacetime_db::client::Client as StdbClient;
use tokio::runtime::Runtime;

mod common;

const LISTING_COUNT: usize = 10;

#[tokio::test]
async fn test_listings_end_to_end() {
    let runtime = Arc::new(Mutex::new(Runtime::new().unwrap()));
    let (backend_addr, backend_thread) = start_backend(&runtime).await;
    let client = Client::new();

    // Create listings
    for i in 0..LISTING_COUNT {
        let response = client
            .post(format!("http://{}/listings", backend_addr))
            .json(&common::create_listing(i))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    }

    // List all listings
    let response = client.get(format!("http://{}/listings", backend_addr)).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let listings: Vec<common::Listing> = response.json().await.unwrap();
    assert_eq!(listings.len(), LISTING_COUNT);

    // Stop backend
    drop(backend_thread);
}

async fn start_backend(runtime: &Arc<Mutex<Runtime>>) -> (SocketAddr, thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();

    let handle = thread::spawn(move || {
        let mut rt = runtime.lock().unwrap();
        rt.block_on(async move {
            // Initialize STDB client and publish module
            let stdb_client = StdbClient::new("temp_module", "ephemeral_key").await.unwrap();
            stdb_client.publish_module("marketplace_module").await.unwrap();

            // Start backend server
            common::start_server(&stdb_client, &listener).await;
        });
    });

    (addr, handle)
}