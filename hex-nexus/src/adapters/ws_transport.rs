//! WebSocket transport adapter for bidirectional agent communication (ADR-040).
//!
//! Implements `IAgentTransportPort` using `tokio-tungstenite`. Manages WebSocket
//! connections to remote agents, typically over SSH-tunneled ports.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures::{SinkExt, StreamExt};
use tokio::sync::{mpsc, RwLock};
use tokio::task::JoinHandle;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tracing;

use crate::ports::agent_transport::IAgentTransportPort;
use crate::remote::transport::{AgentMessage, TransportError};

const CHANNEL_BUFFER: usize = 256;

struct AgentConnection {
    /// Send outgoing messages to the write task
    outgoing_tx: mpsc::Sender<AgentMessage>,
    /// Kept alive so the read loop's clone of this sender doesn't become the only reference.
    /// When the connection is dropped, this sender closes, signaling the read loop to stop.
    #[allow(dead_code)]
    incoming_tx: mpsc::Sender<AgentMessage>,
    /// The receiver end, taken once by `subscribe()`
    incoming_rx: Option<mpsc::Receiver<AgentMessage>>,
    /// Handle to the spawned read loop
    _read_task: JoinHandle<()>,
    /// Handle to the spawned write loop
    _write_task: JoinHandle<()>,
    /// Whether the connection is still alive
    connected: Arc<std::sync::atomic::AtomicBool>,
}

pub struct WsTransportAdapter {
    connections: Arc<RwLock<HashMap<String, AgentConnection>>>,
}

impl WsTransportAdapter {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Accept an incoming WebSocket connection from an agent.
    /// Called when a remote agent connects to the nexus WS endpoint.
    /// Takes ownership of the WS stream and spawns read/write tasks.
    pub async fn accept_connection<S>(
        &self,
        agent_id: String,
        ws_stream: S,
    ) -> Result<(), TransportError>
    where
        S: futures::Stream<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
            + futures::Sink<WsMessage, Error = tokio_tungstenite::tungstenite::Error>
            + Send
            + Unpin
            + 'static,
    {
        // Remove any stale connection for this agent
        {
            let mut conns = self.connections.write().await;
            if let Some(old) = conns.remove(&agent_id) {
                old._read_task.abort();
                old._write_task.abort();
                tracing::info!(agent_id = %agent_id, "replaced stale connection");
            }
        }

        let (write_half, read_half) = ws_stream.split();

        let (outgoing_tx, outgoing_rx) = mpsc::channel::<AgentMessage>(CHANNEL_BUFFER);
        let (incoming_tx, incoming_rx) = mpsc::channel::<AgentMessage>(CHANNEL_BUFFER);

        let connected = Arc::new(std::sync::atomic::AtomicBool::new(true));

        // Spawn write loop
        let write_task = {
            let agent_id = agent_id.clone();
            let connected = connected.clone();
            tokio::spawn(Self::write_loop(agent_id, outgoing_rx, write_half, connected))
        };

        // Spawn read loop
        let read_task = {
            let agent_id = agent_id.clone();
            let incoming_tx = incoming_tx.clone();
            let connected = connected.clone();
            let connections = self.connections.clone();
            tokio::spawn(Self::read_loop(
                agent_id,
                read_half,
                incoming_tx,
                connected,
                connections,
            ))
        };

        let conn = AgentConnection {
            outgoing_tx,
            incoming_tx,
            incoming_rx: Some(incoming_rx),
            _read_task: read_task,
            _write_task: write_task,
            connected,
        };

        self.connections.write().await.insert(agent_id.clone(), conn);
        tracing::info!(agent_id = %agent_id, "accepted WebSocket connection");

        Ok(())
    }

    /// Connect to a remote agent's WebSocket endpoint (client mode).
    /// Used when nexus initiates connection to an agent via SSH tunnel.
    pub async fn connect_to_agent(
        &self,
        agent_id: String,
        url: &str,
    ) -> Result<(), TransportError> {
        tracing::info!(agent_id = %agent_id, url = %url, "connecting to remote agent");

        let (ws_stream, _response) = tokio_tungstenite::connect_async(url)
            .await
            .map_err(|e| TransportError::WebSocket(format!("connect failed: {e}")))?;

        self.accept_connection(agent_id, ws_stream).await
    }

    async fn write_loop<W>(
        agent_id: String,
        mut outgoing_rx: mpsc::Receiver<AgentMessage>,
        mut write_half: W,
        connected: Arc<std::sync::atomic::AtomicBool>,
    ) where
        W: futures::Sink<WsMessage, Error = tokio_tungstenite::tungstenite::Error>
            + Send
            + Unpin
            + 'static,
    {
        while let Some(msg) = outgoing_rx.recv().await {
            let json = match serde_json::to_string(&msg) {
                Ok(j) => j,
                Err(e) => {
                    tracing::error!(agent_id = %agent_id, error = %e, "failed to serialize message");
                    continue;
                }
            };

            if let Err(e) = write_half.send(WsMessage::Text(json)).await {
                tracing::warn!(agent_id = %agent_id, error = %e, "write failed, closing");
                connected.store(false, std::sync::atomic::Ordering::SeqCst);
                break;
            }
        }

        tracing::debug!(agent_id = %agent_id, "write loop ended");
    }

    async fn read_loop<R>(
        agent_id: String,
        mut read_half: R,
        incoming_tx: mpsc::Sender<AgentMessage>,
        connected: Arc<std::sync::atomic::AtomicBool>,
        connections: Arc<RwLock<HashMap<String, AgentConnection>>>,
    ) where
        R: futures::Stream<Item = Result<WsMessage, tokio_tungstenite::tungstenite::Error>>
            + Send
            + Unpin
            + 'static,
    {
        loop {
            match read_half.next().await {
                Some(Ok(WsMessage::Text(text))) => {
                    match serde_json::from_str::<AgentMessage>(&text) {
                        Ok(msg) => {
                            if incoming_tx.send(msg).await.is_err() {
                                tracing::debug!(
                                    agent_id = %agent_id,
                                    "incoming channel closed, stopping read loop"
                                );
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                agent_id = %agent_id,
                                error = %e,
                                "failed to deserialize WS message"
                            );
                        }
                    }
                }
                Some(Ok(WsMessage::Close(_))) => {
                    tracing::info!(agent_id = %agent_id, "received WS close frame");
                    break;
                }
                Some(Ok(WsMessage::Ping(data))) => {
                    // Pong is handled automatically by tungstenite
                    tracing::trace!(agent_id = %agent_id, len = data.len(), "received ping");
                }
                Some(Ok(_)) => {
                    // Binary, Pong, Frame — ignore
                }
                Some(Err(e)) => {
                    tracing::warn!(agent_id = %agent_id, error = %e, "WS read error");
                    break;
                }
                None => {
                    tracing::info!(agent_id = %agent_id, "WS stream ended");
                    break;
                }
            }
        }

        // Mark disconnected and clean up
        connected.store(false, std::sync::atomic::Ordering::SeqCst);
        connections.write().await.remove(&agent_id);
        tracing::info!(agent_id = %agent_id, "connection cleaned up after disconnect");
    }
}

#[async_trait]
impl IAgentTransportPort for WsTransportAdapter {
    async fn send(&self, agent_id: &str, message: AgentMessage) -> Result<(), TransportError> {
        let conns = self.connections.read().await;
        let conn = conns.get(agent_id).ok_or_else(|| {
            TransportError::ConnectionLost(format!("no connection for agent '{agent_id}'"))
        })?;

        if !conn.connected.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(TransportError::ConnectionLost(format!(
                "agent '{agent_id}' is disconnected"
            )));
        }

        conn.outgoing_tx.send(message).await.map_err(|_| {
            TransportError::ConnectionLost(format!(
                "outgoing channel closed for agent '{agent_id}'"
            ))
        })
    }

    async fn subscribe(
        &self,
        agent_id: &str,
    ) -> Result<mpsc::Receiver<AgentMessage>, TransportError> {
        let mut conns = self.connections.write().await;
        let conn = conns.get_mut(agent_id).ok_or_else(|| {
            TransportError::ConnectionLost(format!("no connection for agent '{agent_id}'"))
        })?;

        conn.incoming_rx.take().ok_or_else(|| {
            TransportError::Protocol(format!(
                "subscribe already called for agent '{agent_id}'"
            ))
        })
    }

    async fn is_connected(&self, agent_id: &str) -> bool {
        let conns = self.connections.read().await;
        conns
            .get(agent_id)
            .map(|c| c.connected.load(std::sync::atomic::Ordering::SeqCst))
            .unwrap_or(false)
    }

    async fn disconnect(&self, agent_id: &str) -> Result<(), TransportError> {
        let mut conns = self.connections.write().await;
        if let Some(conn) = conns.remove(agent_id) {
            conn.connected
                .store(false, std::sync::atomic::Ordering::SeqCst);
            conn._read_task.abort();
            conn._write_task.abort();
            tracing::info!(agent_id = %agent_id, "disconnected agent");
            Ok(())
        } else {
            Err(TransportError::ConnectionLost(format!(
                "no connection for agent '{agent_id}'"
            )))
        }
    }
}
