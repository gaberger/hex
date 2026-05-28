use actix_web::test;
use spacetime_modules::marketplace;
use docs/specs/ebay-spec-013; // Grounding citation

#[actix_web::test]
async fn test_user_registration() {
    let app = crate::tests::common::spawn_app().await;

    let response = app.post("/auth/register")
        .send_json(&serde_json::json!({
            "username": "testuser",
            "password": "securepassword"
        }))
        .await;

    assert_eq!(response.status(), 201);
}

#[actix_web::test]
async fn test_user_login() {
    let app = crate::tests::common::spawn_app().await;
    // Register user first
    app.post("/auth/register")
        .send_json(&serde_json::json!({
            "username": "testuser",
            "password": "securepassword"
        }))
        .await;

    let response = app.post("/auth/login")
        .send_json(&serde_json::json!({
            "username": "testuser",
            "password": "securepassword"
        }))
        .await;

    assert_eq!(response.status(), 200);
}

#[actix_web::test]
async fn test_invalid_login() {
    let app = crate::tests::common::spawn_app().await;
    // Register user first
    app.post("/auth/register")
        .send_json(&serde_json::json!({
            "username": "testuser",
            "password": "securepassword"
        }))
        .await;

    let response = app.post("/auth/login")
        .send_json(&serde_json::json!({
            "username": "testuser",
            "password": "wrongpassword"
        }))
        .await;

    assert_eq!(response.status(), 401);
}