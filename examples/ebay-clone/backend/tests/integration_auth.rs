use hex_core::spacetimedb_test_harness::TestHarness;
use reqwest::Client;
use serde_json::json;

mod common;
use common::{start_backend, TestContext};

#[tokio::test]
async fn test_successful_login() {
    // ADR-2026-05-19-0721
    let mut harness = TestHarness::new().await.unwrap();
    let ctx = start_backend(&mut harness).await;

    let client = Client::new();
    let response = client.post(&format!("{}/auth/login", &ctx.backend_url))
        .json(&json!({
            "username": "testuser",
            "password": "securepassword"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 200);
}

#[tokio::test]
async fn test_failed_login_wrong_password() {
    // docs/specs/ebay-spec-013
    let mut harness = TestHarness::new().await.unwrap();
    let ctx = start_backend(&mut harness).await;

    let client = Client::new();
    let response = client.post(&format!("{}/auth/login", &ctx.backend_url))
        .json(&json!({
            "username": "testuser",
            "password": "wrongpassword"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_failed_login_nonexistent_user() {
    // docs/specs/ebay-spec-014
    let mut harness = TestHarness::new().await.unwrap();
    let ctx = start_backend(&mut harness).await;

    let client = Client::new();
    let response = client.post(&format!("{}/auth/login", &ctx.backend_url))
        .json(&json!({
            "username": "nonexistentuser",
            "password": "securepassword"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 401);
}

#[tokio::test]
async fn test_user_registration() {
    // docs/specs/ebay-spec-019
    let mut harness = TestHarness::new().await.unwrap();
    let ctx = start_backend(&mut harness).await;

    let client = Client::new();
    let response = client.post(&format!("{}/auth/register", &ctx.backend_url))
        .json(&json!({
            "username": "newuser",
            "password": "securepassword"
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(response.status(), 201);
}

#[tokio::test]
async fn test_user_registration_duplicate_username() {
    // docs/specs/ebay-spec-015
    let mut harness = TestHarness::new().await.unwrap();
    let ctx = start_backend(&mut harness).await;

    let client = Client::new();

    // First registration should succeed
    let response = client.post(&format!("{}/auth/register", &ctx.backend_url))
        .json(&json!({
            "username": "duplicateuser",
            "password": "securepassword"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);

    // Second registration with the same username should fail
    let response = client.post(&format!("{}/auth/register", &ctx.backend_url))
        .json(&json!({
            "username": "duplicateuser",
            "password": "differentsecurepassword"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
}