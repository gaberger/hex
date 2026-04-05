//! In-memory tool-call event adapter (ADR-2604012137, ADR-2604020900).
//!
//! Replaces the former SQLite-backed SqliteEventAdapter. Events are ephemeral
//! (WebSocket broadcast is the primary delivery mechanism) — a bounded ring
//! buffer of the last 1000 events is sufficient for dashboard history.

use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::ports::events::{InsertEventRequest, ToolEvent};

const RING_CAPACITY: usize = 1000;

/// In-memory ring-buffer event adapter. Thread-safe, no external dependencies.
pub struct InMemoryEventAdapter {
    events: Arc<Mutex<VecDeque<ToolEvent>>>,
    next_id: Arc<Mutex<i64>>,
}

impl InMemoryEventAdapter {
    pub fn new() -> Self {
        Self {
            events: Arc::new(Mutex::new(VecDeque::with_capacity(RING_CAPACITY))),
            next_id: Arc::new(Mutex::new(1)),
        }
    }

    /// Insert one event. Returns `(id, created_at)`.
    pub async fn insert_event(&self, req: &InsertEventRequest) -> (i64, String) {
        let mut id_guard = self.next_id.lock().await;
        let id = *id_guard;
        *id_guard += 1;
        drop(id_guard);

        let created_at = chrono::Utc::now().to_rfc3339();
        let event = ToolEvent {
            id,
            session_id: req.session_id.clone(),
            agent_id: req.agent_id.clone(),
            event_type: req.event_type.clone(),
            tool_name: req.tool_name.clone(),
            input_json: req.input_json.as_deref().map(truncate_4kb).map(str::to_owned),
            result_json: req.result_json.as_deref().map(truncate_4kb).map(str::to_owned),
            exit_code: req.exit_code,
            duration_ms: req.duration_ms,
            model_used: req.model_used.clone(),
            context_strategy: req.context_strategy.clone(),
            rl_action: req.rl_action.clone(),
            input_tokens: req.input_tokens,
            output_tokens: req.output_tokens,
            cost_usd: req.cost_usd,
            hex_layer: req.hex_layer.clone(),
            created_at: created_at.clone(),
        };

        let mut buf = self.events.lock().await;
        if buf.len() >= RING_CAPACITY {
            buf.pop_front();
        }
        buf.push_back(event);

        (id, created_at)
    }

    /// List events, newest first, optionally filtered by session_id.
    pub async fn list_events(&self, session_id: Option<&str>, limit: u32) -> Vec<ToolEvent> {
        let limit = (limit.min(500)) as usize;
        let buf = self.events.lock().await;
        buf.iter()
            .rev()
            .filter(|e| session_id.is_none_or(|sid| e.session_id == sid))
            .take(limit)
            .cloned()
            .collect()
    }
}

impl Default for InMemoryEventAdapter {
    fn default() -> Self {
        Self::new()
    }
}

fn truncate_4kb(s: &str) -> &str {
    const MAX: usize = 4096;
    if s.len() <= MAX {
        return s;
    }
    let mut end = MAX;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}
