use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        Query, State,
    },
    response::IntoResponse,
};
use futures::{SinkExt, StreamExt};
use serde::Deserialize;
use serde_json::json;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::state::{SharedState, WsEnvelope};

#[derive(Debug, Deserialize)]
pub struct WsParams {
    pub token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum InboundMessage {
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Publish { topic: String, event: String, data: Option<serde_json::Value> },
}

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
    Query(params): Query<WsParams>,
) -> impl IntoResponse {
    let authenticated = match &state.auth_token {
        None => true,
        Some(token) => params.token.as_deref() == Some(token.as_str()),
    };

    ws.on_upgrade(move |socket| handle_ws(socket, state, authenticated))
}

async fn handle_ws(socket: WebSocket, state: SharedState, authenticated: bool) {
    let (mut sender, mut receiver) = socket.split();
    let client_id = Uuid::new_v4().to_string();
    let subscriptions: Arc<Mutex<HashSet<String>>> = Arc::new(Mutex::new(HashSet::new()));

    // Send welcome message
    let welcome = serde_json::to_string(&WsEnvelope {
        topic: "hub:health".to_string(),
        event: "connected".to_string(),
        data: json!({ "clientId": client_id, "authenticated": authenticated }),
    })
    .unwrap();
    let _ = sender.send(Message::Text(welcome.into())).await;

    // Subscribe to broadcast channel for fan-out
    let mut ws_rx = state.ws_tx.subscribe();
    let subs_clone = subscriptions.clone();

    // Task: forward matching broadcast messages to this client
    let mut send_task = tokio::spawn(async move {
        loop {
            match ws_rx.recv().await {
                Ok(envelope) => {
                    let subs = subs_clone.lock().await;
                    let matches = subs.iter().any(|pattern| topic_matches(pattern, &envelope.topic));
                    drop(subs);

                    if matches {
                        let msg = serde_json::to_string(&envelope).unwrap_or_default();
                        if sender.send(Message::Text(msg.into())).await.is_err() {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    // Task: handle inbound messages from client
    let state2 = state.clone();
    let subs2 = subscriptions.clone();
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Ping(_) => continue,
                Message::Pong(_) => continue,
                Message::Close(_) => break,
                _ => continue,
            };

            let parsed: InboundMessage = match serde_json::from_str(&text) {
                Ok(m) => m,
                Err(_) => {
                    // Send error — but we don't have sender here.
                    // The WS protocol says invalid JSON is ignored silently.
                    continue;
                }
            };

            match parsed {
                InboundMessage::Subscribe { topic } => {
                    subs2.lock().await.insert(topic);
                }
                InboundMessage::Unsubscribe { topic } => {
                    subs2.lock().await.remove(&topic);
                }
                InboundMessage::Publish { topic, event, data } => {
                    if state2.auth_token.is_some() && !authenticated {
                        // Can't publish without auth — drop silently
                        // (error message would need the sender half)
                        continue;
                    }
                    let envelope = WsEnvelope {
                        topic,
                        event,
                        data: data.unwrap_or(serde_json::Value::Null),
                    };
                    let _ = state2.ws_tx.send(envelope);
                }
            }
        }
    });

    // Wait for either task to finish, then abort the other
    tokio::select! {
        _ = &mut send_task => recv_task.abort(),
        _ = &mut recv_task => send_task.abort(),
    }
}

/// Match topic against subscription pattern.
/// Supports trailing `:*` wildcard: `project:abc:*` matches `project:abc:file-change`.
fn topic_matches(pattern: &str, topic: &str) -> bool {
    if pattern == topic {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        topic.starts_with(prefix)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match() {
        assert!(topic_matches("hub:health", "hub:health"));
    }

    #[test]
    fn wildcard_match() {
        assert!(topic_matches("project:abc:*", "project:abc:file-change"));
        assert!(topic_matches("project:abc:*", "project:abc:task-progress"));
    }

    #[test]
    fn wildcard_no_match() {
        assert!(!topic_matches("project:abc:*", "project:def:file-change"));
    }

    #[test]
    fn no_partial_match_without_wildcard() {
        assert!(!topic_matches("project:abc", "project:abc:file-change"));
    }
}
