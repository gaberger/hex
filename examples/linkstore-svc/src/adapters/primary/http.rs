//! The HTTP front door: a router and four handlers.
//!
//! This adapter imports `ports/` only. It turns text into a port call, and a
//! port answer into a status code.

use crate::adapters::primary::wire::{BookmarkResponse, CreateRequest, ErrorBody, ListQuery};
use crate::ports::{ApiError, BookmarkApi, BookmarkValue};
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::sync::Arc;

type Api = Arc<dyn BookmarkApi>;

/// axum 0.8 route syntax is `{id}`. The old `:id` form does not just fail to
/// match — the router **panics at startup**, and every test then dies with a
/// routing error that looks nothing like the real bug.
pub fn router(api: Api) -> Router {
    Router::new()
        .route("/bookmarks", post(create).get(list))
        .route("/bookmarks/{id}", get(fetch).delete(remove))
        .with_state(api)
}

/// Run one port call off the web server's threads.
///
/// SQLite blocks the thread it runs on, and tokio has a small fixed number of
/// worker threads. It is like a restaurant where every waiter who takes an
/// order must stand in the kitchen until the food is cooked: four slow orders
/// and nobody greets the door. This moves the cooking elsewhere.
async fn off_thread<T, F>(work: F) -> Result<T, ApiError>
where
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
    T: Send + 'static,
{
    match tokio::task::spawn_blocking(work).await {
        Ok(result) => result,
        Err(join_error) => {
            eprintln!("the blocking task failed: {join_error}");
            Err(ApiError::Unavailable)
        }
    }
}

/// One typed error becomes one status code. Because `ApiError` is an enum and
/// not a string, no status code here depends on the wording of a message.
fn to_response(error: ApiError) -> Response {
    match error {
        ApiError::Invalid(why) => (StatusCode::BAD_REQUEST, Json(ErrorBody::new(why))),
        ApiError::NotFound => {
            (StatusCode::NOT_FOUND, Json(ErrorBody::new("not found")))
        }
        // The body never carries SQL text or a file path. The log does.
        ApiError::Unavailable => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorBody::new("store unavailable")),
        ),
    }
    .into_response()
}

fn created(bookmark: &BookmarkValue) -> Response {
    let body = BookmarkResponse::from_port(bookmark);
    let location = format!("/bookmarks/{}", body.id);
    (StatusCode::CREATED, [(header::LOCATION, location)], Json(body)).into_response()
}

async fn create(State(api): State<Api>, Json(request): Json<CreateRequest>) -> Response {
    let work = move || api.create(&request.url, &request.title, &request.tags);
    match off_thread(work).await {
        // A repeat save answers 201 with the same identity. The brief says
        // POST returns 201, and the brief is the contract — not whatever
        // number is convenient once the code exists.
        Ok(bookmark) => created(&bookmark),
        Err(error) => to_response(error),
    }
}

async fn fetch(State(api): State<Api>, Path(id): Path<String>) -> Response {
    let work = move || api.get(&id);
    match off_thread(work).await {
        Ok(bookmark) => (StatusCode::OK, Json(BookmarkResponse::from_port(&bookmark))).into_response(),
        Err(error) => to_response(error),
    }
}

async fn list(State(api): State<Api>, Query(query): Query<ListQuery>) -> Response {
    let Some(tag) = query.tag else {
        return to_response(ApiError::Invalid("a tag is required".into()));
    };
    let limit = query.limit;
    let work = move || api.list_by_tag(&tag, limit);
    match off_thread(work).await {
        Ok(bookmarks) => {
            let body: Vec<BookmarkResponse> =
                bookmarks.iter().map(BookmarkResponse::from_port).collect();
            (StatusCode::OK, Json(body)).into_response()
        }
        Err(error) => to_response(error),
    }
}

/// Always 204, including for an identity that was never valid.
///
/// State the cost plainly: from outside, a client's typo and a real delete
/// look the same. Both callers are right about how the world ends up.
async fn remove(State(api): State<Api>, Path(id): Path<String>) -> Response {
    let work = move || api.delete(&id);
    match off_thread(work).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => to_response(error),
    }
}
