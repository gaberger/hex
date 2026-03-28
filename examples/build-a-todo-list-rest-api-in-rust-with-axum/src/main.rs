use axum::prelude::*;
use axum::routing::route;
use axum::routing::get;

async fn hello() -> String {
    "Hello, World!".into()
}

#[tokio::main]
async fn main() {
    let app = route("/hello", get(hello)).to_app();
    app.bind("127.0.0.1:3000").await.unwrap();
}