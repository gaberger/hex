//! Chat session persistence — auto-save / resume for `hex chat`.
//!
//! Sessions are stored as JSON in `~/.hex/sessions/chat-{uuid}.json`.

use std::path::PathBuf;

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct ChatSession {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub project_id: String,
    pub messages: Vec<SessionMessage>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SessionMessage {
    pub role: String,
    pub content: String,
}

impl ChatSession {
    pub fn new(model: &str, project_id: &str) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: now.clone(),
            updated_at: now,
            model: model.to_string(),
            project_id: project_id.to_string(),
            messages: Vec::new(),
        }
    }

    pub fn save(&self) -> Result<()> {
        let dir = sessions_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("chat-{}.json", self.id));
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn load(id: &str) -> Result<Self> {
        let path = sessions_dir().join(format!("chat-{}.json", id));
        let json = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&json)?)
    }

    /// Return sessions sorted by `updated_at` descending.
    pub fn list_recent(limit: usize) -> Result<Vec<ChatSession>> {
        let dir = sessions_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions: Vec<ChatSession> = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(raw) = std::fs::read_to_string(&path) {
                if let Ok(s) = serde_json::from_str::<ChatSession>(&raw) {
                    sessions.push(s);
                }
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        sessions.truncate(limit);
        Ok(sessions)
    }

    /// First user message, truncated for display.
    pub fn preview(&self) -> String {
        self.messages
            .iter()
            .find(|m| m.role == "user")
            .map(|m| {
                let mut s: String = m.content.chars().take(70).collect();
                if m.content.chars().count() > 70 {
                    s.push('…');
                }
                s
            })
            .unwrap_or_else(|| "(empty)".to_string())
    }
}

pub fn sessions_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_default()
        .join(".hex")
        .join("sessions")
}
