use axum::{Router, routing::{get, post, delete}, middleware, response::{IntoResponse, Json, Response}, extract::{State, Path}, http::StatusCode};
use adapters::primary::router::app;
use std::net::TcpListener;

type AppState = ();

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let app = Router::new()
        .route("/todos", get(app::get_todos).post(app::create_todo).delete(app::delete_todo))
        .route("/todos/:id", delete(app::delete_todo))
        .layer(middleware::from_fn_with_state(AppState, middleware::trace));
    
    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    axum::serve(listener, app).await?;
    Ok(())
}