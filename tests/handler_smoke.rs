use reqwest::Client;
use serde_json::json;

#[tokio::test]
async fn test_handler_happy_path() {
    // ADR-2026-05-19-0721
    let client = Client::new();
    let url = "http://localhost:8000/api/handler";

    // Test with valid input
    let payload = json!({
        "field": "value"
    });

    let response = client.post(url)
        .json(&payload)
        .send()
        .await
        .expect("Request failed");

    assert_eq!(response.status(), 200);

    // Test with missing field
    let invalid_payload = json!({});

    let error_response = client.post(url)
        .json(&invalid_payload)
        .send()
        .await
        .expect("Request failed");

    assert_eq!(error_response.status(), 400);
}