use actix_web::{web, App, HttpResponse, HttpServer, Responder};
use serde::Deserialize;
use validator::Validate;

#[derive(Deserialize, Validate)]
struct HandlerInput {
    #[validate(length(min = 1))]
    field: String,
}

async fn handler(data: web::Json<HandlerInput>) -> impl Responder {
    if let Err(e) = data.validate() {
        return HttpResponse::BadRequest().body(format!("Validation error: {}", e));
    }
    HttpResponse::Ok().body("Received valid input")
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    HttpServer::new(|| {
        App::new()
            .route("/api/handler", web::post().to(handler))
    })
    .bind("127.0.0.1:8080")?
    .run()
    .await
}

// ADR-2026-05-19-0721