use actix_web::{test, App, web};
use hex_core::api::handler; // Assuming handler is defined in this module

#[actix_web::test]
async fn test_handler_happy_path() {
    let app = test::init_service(
        App::new().route("/api/handler", web::post().to(handler)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/api/handler")
        .set_json(serde_json::json!({"field": "value"}))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert!(resp.status().is_success());
}

#[actix_web::test]
async fn test_handler_missing_field() {
    let app = test::init_service(
        App::new().route("/api/handler", web::post().to(handler)),
    )
    .await;

    let req = test::TestRequest::post()
        .uri("/api/handler")
        .set_json(serde_json::json!({}))
        .to_request();

    let resp = test::call_service(&app, req).await;
    assert_eq!(resp.status(), 400);
}