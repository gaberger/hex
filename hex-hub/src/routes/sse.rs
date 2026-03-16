use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::Stream;
use serde_json::json;
use std::{convert::Infallible, time::Duration};

use crate::state::{SharedState, SseParams};

pub async fn sse_handler(
    State(state): State<SharedState>,
    Query(params): Query<SseParams>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let mut rx = state.sse_tx.subscribe();
    let project_filter = params.project;

    let stream = async_stream::stream! {
        // Send initial project list on connect
        {
            let projects = state.projects.read().await;
            let list: Vec<serde_json::Value> = projects.values().map(|p| {
                json!({
                    "id": p.id,
                    "name": p.name,
                    "rootPath": p.root_path,
                    "astIsStub": p.state.project.as_ref().map(|m| m.ast_is_stub).unwrap_or(false),
                })
            }).collect();
            yield Ok(Event::default()
                .event("connected")
                .data(serde_json::to_string(&json!({ "projects": list })).unwrap()));
        }

        // Stream events
        loop {
            match rx.recv().await {
                Ok(sse_event) => {
                    // Apply project filter
                    let should_send = match (&project_filter, &sse_event.project_id) {
                        (None, _) => true,                     // No filter: send all
                        (Some(f), Some(p)) => f == p,          // Filter matches project
                        (Some(_), None) => true,               // Global events always pass
                    };

                    if should_send {
                        let data = serde_json::to_string(&sse_event.data).unwrap_or_default();
                        yield Ok(Event::default()
                            .event(&sse_event.event_type)
                            .data(data));
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    };

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("heartbeat"),
    )
}
