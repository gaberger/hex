//! Persona supervisor client — thin STDB shim used by org_responder to drive
//! the OTP-style persona supervisor primitives in the `hex` database
//! (hexflo-coordination module).
//!
//! Responsibilities:
//!   - `is_banned(role)`: read `persona_health.banned_until` and answer
//!     "should the responder skip this role right now?"
//!   - `record_failure(role, model, status_code)`: bump the rolling-window
//!     failure counter; STDB-side reducer auto-bans after threshold.
//!   - `record_success(role)`: clear the counter + any active ban.
//!
//! All three operations are best-effort — failures here MUST NOT prevent
//! the responder from making a reply. We log and proceed.

use std::time::Duration;

#[derive(Clone)]
pub struct SpacetimePersonaSupervisor {
    http: reqwest::Client,
    host: String,
    /// `hex` database (hexflo-coordination module) — where persona_pool /
    /// persona_health / persona_event live.
    database: String,
    /// `chat-relay` database — where `agent_thought` lives. Optional;
    /// journaling silently no-ops if unset.
    chat_relay_database: Option<String>,
}

impl SpacetimePersonaSupervisor {
    pub fn new(host: String, database: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .pool_max_idle_per_host(2)
            .build()
            .expect("failed to build HTTP client");
        Self { http, host, database, chat_relay_database: None }
    }

    pub fn with_chat_relay(mut self, db: String) -> Self {
        self.chat_relay_database = Some(db);
        self
    }

    /// Returns `Some(banned_until_ts)` if the role is currently banned,
    /// `None` otherwise. Treats any error as "not banned" to avoid amplifying
    /// outages — if STDB is down, let the responder try the inference call.
    pub async fn is_banned(&self, role: &str) -> Option<String> {
        let safe_role = role.replace('\'', "''");
        let q = format!(
            "SELECT banned_until FROM persona_health WHERE role = '{}'",
            safe_role
        );
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);
        let resp = match self
            .http
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(q)
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => return None,
        };
        if !resp.status().is_success() {
            return None;
        }
        let body: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return None,
        };
        let rows = body
            .as_array()?
            .first()?
            .get("rows")?
            .as_array()?;
        let banned_until = rows
            .first()?
            .as_array()?
            .first()?
            .as_str()?
            .to_string();
        if banned_until.is_empty() {
            return None;
        }
        // banned_until is `Timestamp { __timestamp_micros_since_unix_epoch__: N }`.
        // If N > now, persona is banned. Parse + compare.
        let now_micros = chrono::Utc::now().timestamp_micros();
        match parse_ts_micros(&banned_until) {
            Some(b) if b > now_micros => Some(banned_until),
            _ => None,
        }
    }

    pub async fn record_failure(&self, role: &str, model_id: &str, status_code: u16) {
        let url = format!(
            "{}/v1/database/{}/call/persona_record_inference_failure",
            self.host, self.database
        );
        let body = serde_json::json!([role, model_id, status_code as u32]);
        if let Err(e) = self.http.post(&url).json(&body).send().await {
            tracing::debug!(role = %role, error = %e, "persona record_failure: transport error");
        }
    }

    pub async fn record_success(&self, role: &str) {
        let url = format!(
            "{}/v1/database/{}/call/persona_record_inference_success",
            self.host, self.database
        );
        let body = serde_json::json!([role]);
        if let Err(e) = self.http.post(&url).json(&body).send().await {
            tracing::debug!(role = %role, error = %e, "persona record_success: transport error");
        }
    }

    /// Journal an agent thought (kind=decision/observation/plan/frustration/learning/commitment).
    /// Best-effort: failures here MUST NOT block the responder. Silently
    /// no-ops if `chat_relay_database` is not configured.
    pub async fn journal_thought(
        &self,
        role: &str,
        kind: &str,
        content: &str,
        related_task_id: &str,
        related_msg_id: u64,
        confidence: f32,
    ) {
        let Some(ref db) = self.chat_relay_database else {
            return;
        };
        let url = format!("{}/v1/database/{}/call/journal_thought", self.host, db);
        let body = serde_json::json!([
            role,
            kind,
            content,
            related_task_id,
            related_msg_id,
            confidence,
        ]);
        if let Err(e) = self.http.post(&url).json(&body).send().await {
            tracing::debug!(
                role = %role, kind = %kind, error = %e,
                "persona journal_thought: transport error"
            );
        }
    }
}

fn parse_ts_micros(s: &str) -> Option<i64> {
    let key = "__timestamp_micros_since_unix_epoch__:";
    let pos = s.find(key)?;
    let tail = &s[pos + key.len()..];
    let end = tail
        .find(|c: char| !c.is_ascii_digit() && c != '-' && c != ' ')
        .unwrap_or(tail.len());
    tail[..end].trim().parse::<i64>().ok()
}
