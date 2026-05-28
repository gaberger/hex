use spacetimedb::auth::{AuthKey, Permissions};
use spacetimedb::reducer::ReducerCallPort;
use spacetimedb::test_utils::{StableDbHarness, TestHarness};
use reqwest::Client;
use serde_json::json;

mod common;
use crate::common::{setup_test_harness, teardown_test_harness};

#[tokio::test]
async fn test_image_upload() {
    let harness = setup_test_harness().await;
    let client = Client::new();

    // Simulate image upload
    let image_data = b"fake image data";
    let response = client.post(&format!("{}/api/upload_image", harness.api_url()))
        .header("Content-Type", "image/jpeg")
        .body(image_data.to_vec())
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);

    teardown_test_harness(harness).await;
}

#[tokio::test]
async fn test_image_upload_negative() {
    let harness = setup_test_harness().await;
    let client = Client::new();

    // Simulate image upload with invalid content type
    let image_data = b"fake image data";
    let response = client.post(&format!("{}/api/upload_image", harness.api_url()))
        .header("Content-Type", "text/plain")
        .body(image_data.to_vec())
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 400);

    teardown_test_harness(harness).await;
}

// This test covers spec references: docs/specs/ebay-spec-013, docs/specs/ebay-spec-014, docs/specs/ebay-spec-015
#[tokio::test]
async fn test_image_reducer_calls() {
    let harness = setup_test_harness().await;
    let client = Client::new();

    // Simulate image upload and get the image ID
    let image_data = b"fake image data";
    let response = client.post(&format!("{}/api/upload_image", harness.api_url()))
        .header("Content-Type", "image/jpeg")
        .body(image_data.to_vec())
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
    let image_id: serde_json::Value = response.json().await.unwrap();
    let image_id = image_id["id"].as_str().unwrap();

    // Direct ReducerCallPort call to delete the image
    let auth_key = AuthKey::new(harness.stable_db.auth_token().clone(), Permissions::all());
    let port = ReducerCallPort::connect(&harness.stable_db.db_url()).await.unwrap();
    let result = port.call("delete_image", json!({ "image_id": image_id }), &auth_key).await;

    assert!(result.is_ok());

    teardown_test_harness(harness).await;
}