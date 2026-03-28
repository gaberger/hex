use axum::{
    Router,
    routing::{get, post, put, delete},
    extract::{State, Path, Json},
    http::StatusCode,
};
use tokio::net::TcpListener;

#[derive(Clone, Debug)]
pub struct MyState {
    store: Arc<Mutex<HashMap<String, String>>>,
}

#[tokio::main]
async fn main() {
    let state = Arc::new(MyState {
        store: Arc::new(Mutex::new(HashMap::new())),
    });

    let app = Router::new().route("/key", get(get_key))
        .route("/key", post(create_key))
        .route("/key/:key", put(update_key))
        .route("/key/:key", delete(delete_key));

    let listener = TcpListener::bind("0.0.0.0:3000").await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn get_key(State(state): State<MyState>) -> Json<HashMap<String, String>> {
    let store = state.store.lock().unwrap();
    Json(store.clone())
}

async fn create_key(
    State(state): State<MyState>,
    Json(body): Json<CreateRequest>,
) -> (StatusCode, Json<HashMap<String, String>>) {
    let mut store = state.store.lock().unwrap();
    store.insert(body.key.clone(), body.value);
    (StatusCode::CREATED, Json(store.clone()))
}

async fn update_key(
    State(state): State<MyState>,
    Path(key): Path<String>,
    Json(body): Json<UpdateRequest>,
) -> (StatusCode, Json<HashMap<String, String>>) {
    let mut store = state.store.lock().unwrap();
    store.insert(key, body.value);
    (StatusCode::OK, Json(store.clone()))
}

async fn delete_key(
    State(state): State<MyState>,
    Path(key): Path<String>,
) -> (StatusCode, Json<HashMap<String, String>>) {
    let mut store = state.store.lock().unwrap();
    store.remove(&key);
    (StatusCode::OK, Json(store.clone()))
}

#[derive(Deserialize, Debug)]
struct CreateRequest {
    key: String,
    value: String,
}

#[derive(Deserialize, Debug)]
struct UpdateRequest {
    value: String,
}