use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};

use crate::state::SharedState;

/// WebSocket endpoint for real-time inference task push.
/// Agents connect here to receive InferenceTaskPush messages without polling.
/// ADR-2604011200
pub async fn ws_inference_handler(
    ws: WebSocketUpgrade,
    State(state): State<SharedState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_inference_ws(socket, state))
}

async fn handle_inference_ws(mut socket: WebSocket, state: SharedState) {
    let mut rx = state.inference_tx.subscribe();
    loop {
        tokio::select! {
            Ok(push) = rx.recv() => {
                let json = match serde_json::to_string(&push) {
                    Ok(s) => s,
                    Err(_) => continue,
                };
                if socket.send(Message::Text(json.into())).await.is_err() {
                    break; // client disconnected
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {} // ignore pings/pongs/text from client
                }
            }
        }
    }
}
