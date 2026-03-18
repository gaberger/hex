use crate::ports::hub::{HubClientPort, HubError, HubMessage};
use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::Mutex;
use tokio_tungstenite::{
    connect_async,
    tungstenite::Message as WsMessage,
};

type WsSink = futures::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    WsMessage,
>;
type WsStream = futures::stream::SplitStream<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
>;

/// WebSocket adapter connecting hex-agent back to hex-hub.
///
/// Streams output (text chunks, tool calls, token updates) to the hub
/// and receives commands (chat messages, shutdown) from it.
pub struct HubClientAdapter {
    sink: Mutex<Option<WsSink>>,
    stream: Mutex<Option<WsStream>>,
    connected: AtomicBool,
}

impl HubClientAdapter {
    pub fn new() -> Self {
        Self {
            sink: Mutex::new(None),
            stream: Mutex::new(None),
            connected: AtomicBool::new(false),
        }
    }
}

#[async_trait]
impl HubClientPort for HubClientAdapter {
    async fn connect(&self, hub_url: &str, auth_token: &str) -> Result<(), HubError> {
        // Build WebSocket URL: ws(s)://host/ws/agent?token=xxx
        let ws_url = if hub_url.starts_with("https") {
            hub_url.replacen("https", "wss", 1)
        } else {
            hub_url.replacen("http", "ws", 1)
        };

        let url = format!("{}/ws/agent?token={}", ws_url, auth_token);

        let (ws_stream, _response) = connect_async(&url)
            .await
            .map_err(|e| HubError::ConnectionFailed(e.to_string()))?;

        let (sink, stream) = ws_stream.split();

        *self.sink.lock().await = Some(sink);
        *self.stream.lock().await = Some(stream);
        self.connected.store(true, Ordering::SeqCst);

        tracing::info!("Connected to hex-hub WebSocket");
        Ok(())
    }

    async fn send(&self, message: HubMessage) -> Result<(), HubError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(HubError::NotConnected);
        }

        let json = serde_json::to_string(&message)
            .map_err(|e| HubError::SendFailed(e.to_string()))?;

        let mut guard = self.sink.lock().await;
        let sink = guard.as_mut().ok_or(HubError::NotConnected)?;

        sink.send(WsMessage::Text(json.into()))
            .await
            .map_err(|e| {
                self.connected.store(false, Ordering::SeqCst);
                HubError::SendFailed(e.to_string())
            })
    }

    async fn recv(&self) -> Result<HubMessage, HubError> {
        if !self.connected.load(Ordering::SeqCst) {
            return Err(HubError::NotConnected);
        }

        loop {
            // Acquire stream lock per iteration so we can release it for ping responses
            let msg = {
                let mut guard = self.stream.lock().await;
                let stream = guard.as_mut().ok_or(HubError::NotConnected)?;
                stream.next().await
            };

            match msg {
                Some(Ok(WsMessage::Text(text))) => {
                    let msg: HubMessage = serde_json::from_str(&text)
                        .map_err(|e| HubError::ReceiveFailed(e.to_string()))?;
                    return Ok(msg);
                }
                Some(Ok(WsMessage::Ping(data))) => {
                    let mut sink_guard = self.sink.lock().await;
                    if let Some(sink) = sink_guard.as_mut() {
                        let _ = sink.send(WsMessage::Pong(data)).await;
                    }
                    continue;
                }
                Some(Ok(WsMessage::Close(_))) => {
                    self.connected.store(false, Ordering::SeqCst);
                    return Err(HubError::ReceiveFailed("Connection closed by hub".into()));
                }
                Some(Ok(_)) => continue,
                Some(Err(e)) => {
                    self.connected.store(false, Ordering::SeqCst);
                    return Err(HubError::ReceiveFailed(e.to_string()));
                }
                None => {
                    self.connected.store(false, Ordering::SeqCst);
                    return Err(HubError::ReceiveFailed("Stream ended".into()));
                }
            }
        }
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    async fn disconnect(&self) -> Result<(), HubError> {
        let mut sink_guard = self.sink.lock().await;
        if let Some(mut sink) = sink_guard.take() {
            let _ = sink.send(WsMessage::Close(None)).await;
        }
        *self.stream.lock().await = None;
        self.connected.store(false, Ordering::SeqCst);
        tracing::info!("Disconnected from hex-hub");
        Ok(())
    }
}
