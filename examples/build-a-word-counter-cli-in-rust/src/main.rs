use axum::Router;
use axum::routing::get;
use axum::extract::State;
use axum::extract::Json;
use axum::http::StatusCode;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let state = MyState::new();
    let app = Router::new().route("/hello", get(handler));
    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handler(State(state): State<MyState>) -> Json<String> {
    Json(state.greeting())
}

struct MyState {
    greeting: String,
}

impl MyState {
    fn new() -> Self {
        Self {
            greeting: "Hello from Hexagonal Rust!".to_string(),
        }
    }

    fn greeting(&self) -> String {
        self.greeting.clone()
    }
}