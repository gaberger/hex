//! Inference-task push payload + bus (ADR-2606071340 P1).
//!
//! Shared by the `/ws/inference` route producers (in hex-nexus) and the STDB
//! state adapter (hex-state). A plain DTO + a tokio broadcast alias — it lives
//! in hex-core so neither side has to depend on the other's crate.

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

/// Push payload for `/ws/inference` subscribers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceTaskPush {
    pub id: String,
    pub workplan_id: String,
    pub task_id: String,
    pub phase: String,
    pub prompt: String,
    pub role: String,
}

/// Broadcast bus carrying [`InferenceTaskPush`] events to WS subscribers.
pub type InferenceTxBus = broadcast::Sender<InferenceTaskPush>;
