//! SpacetimeDB inference-gateway adapter.
//!
//! Talks to the `inference-gateway` SpacetimeDB module via HTTP to persist
//! inference requests/responses and manage provider registration.
//! All calls are fire-and-forget friendly — failures are logged but never
//! break the hot path.

use serde::{Deserialize, Serialize};
use std::time::Duration;

/// HTTP client for the `inference-gateway` SpacetimeDB module.
pub struct SpacetimeInferenceClient {
    http: reqwest::Client,
    host: String,
    database: String,
}

// ── Response types for SQL queries ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceProviderRow {
    pub provider_id: String,
    pub provider_type: String,
    pub base_url: String,
    pub api_key_ref: String,
    pub models_json: String,
    pub rate_limit_rpm: u32,
    pub rate_limit_tpm: u64,
    pub current_rpm: u32,
    pub current_tpm: u64,
    pub healthy: u8,
    pub last_health_check: String,
    pub avg_latency_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InferenceResponseRow {
    pub response_id: u64,
    pub request_id: u64,
    pub agent_id: String,
    pub status: String,
    pub content_json: String,
    pub model_used: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub latency_ms: u64,
    pub cost_usd: String,
    pub created_at: String,
}

impl SpacetimeInferenceClient {
    pub fn new(host: String, database: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");

        Self {
            http,
            host,
            database,
        }
    }

    /// Get the SpacetimeDB host URL.
    pub fn host_url(&self) -> &str { &self.host }

    /// Get the SpacetimeDB database name.
    pub fn database_name(&self) -> &str { &self.database }

    // ── Reducer calls ───────────────────────────────────────────────────

    /// Register (or update) an inference provider in SpacetimeDB.
    pub async fn register_provider(
        &self,
        provider_id: &str,
        provider_type: &str,
        base_url: &str,
        api_key_ref: &str,
        models_json: &str,
        rate_limit_rpm: u32,
        rate_limit_tpm: u64,
    ) -> Result<(), String> {
        self.call_reducer(
            "register_provider",
            serde_json::json!([
                provider_id,
                provider_type,
                base_url,
                api_key_ref,
                models_json,
                rate_limit_rpm,
                rate_limit_tpm,
            ]),
        )
        .await
    }

    /// Remove an inference provider from SpacetimeDB.
    pub async fn remove_provider(&self, provider_id: &str) -> Result<(), String> {
        self.call_reducer("remove_provider", serde_json::json!([provider_id])).await
    }

    /// Submit an inference request to SpacetimeDB (status = "queued").
    ///
    /// Note: `request_id` is auto-incremented by SpacetimeDB. The caller
    /// should query `inference_request` afterwards to find the assigned ID
    /// if needed — but for audit-only use we skip this.
    pub async fn request_inference(
        &self,
        agent_id: &str,
        provider: &str,
        model: &str,
        messages_json: &str,
        tools_json: &str,
        max_tokens: u32,
        temperature: &str,
        thinking_budget: u32,
        cache_control: u8,
        priority: u8,
        created_at: &str,
    ) -> Result<(), String> {
        self.call_reducer(
            "request_inference",
            serde_json::json!([
                agent_id,
                provider,
                model,
                messages_json,
                tools_json,
                max_tokens,
                temperature,
                thinking_budget,
                cache_control,
                priority,
                created_at,
            ]),
        )
        .await
    }

    /// Record a completed inference response in SpacetimeDB.
    ///
    /// `openrouter_cost_usd` carries the actual cost reported by OpenRouter's
    /// `usage.cost` field.  Pass an empty string for non-OpenRouter providers.
    pub async fn complete_inference(
        &self,
        request_id: u64,
        content_json: &str,
        model_used: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_read_tokens: u64,
        cache_write_tokens: u64,
        latency_ms: u64,
        cost_usd: &str,
        openrouter_cost_usd: &str,
        created_at: &str,
    ) -> Result<(), String> {
        self.call_reducer(
            "complete_inference",
            serde_json::json!([
                request_id,
                content_json,
                model_used,
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
                latency_ms,
                cost_usd,
                openrouter_cost_usd,
                created_at,
            ]),
        )
        .await
    }

    // ── SQL queries ─────────────────────────────────────────────────────

    /// List all registered inference providers from SpacetimeDB.
    pub async fn list_providers(&self) -> Result<Vec<InferenceProviderRow>, String> {
        let rows = self
            .sql_query("SELECT * FROM inference_provider")
            .await?;

        let mut providers = Vec::new();
        for row in rows {
            if let Some(cols) = row.as_array() {
                // Column order matches struct definition:
                // provider_id, provider_type, base_url, api_key_ref, models_json,
                // rate_limit_rpm, rate_limit_tpm, current_rpm, current_tpm,
                // healthy, last_health_check, avg_latency_ms
                if cols.len() >= 12 {
                    providers.push(InferenceProviderRow {
                        provider_id: str_col(cols, 0),
                        provider_type: str_col(cols, 1),
                        base_url: str_col(cols, 2),
                        api_key_ref: str_col(cols, 3),
                        models_json: str_col(cols, 4),
                        rate_limit_rpm: u32_col(cols, 5),
                        rate_limit_tpm: u64_col(cols, 6),
                        current_rpm: u32_col(cols, 7),
                        current_tpm: u64_col(cols, 8),
                        healthy: cols.get(9).and_then(|v| v.as_u64()).unwrap_or(0) as u8,
                        last_health_check: str_col(cols, 10),
                        avg_latency_ms: u64_col(cols, 11),
                    });
                }
            }
        }
        Ok(providers)
    }

    /// Poll for a response to a specific request_id.
    pub async fn poll_response(
        &self,
        request_id: u64,
    ) -> Result<Option<InferenceResponseRow>, String> {
        let query = format!(
            "SELECT * FROM inference_response WHERE request_id = {}",
            request_id
        );
        let rows = self.sql_query(&query).await?;

        if let Some(row) = rows.first() {
            if let Some(cols) = row.as_array() {
                if cols.len() >= 13 {
                    return Ok(Some(InferenceResponseRow {
                        response_id: u64_col(cols, 0),
                        request_id: u64_col(cols, 1),
                        agent_id: str_col(cols, 2),
                        status: str_col(cols, 3),
                        content_json: str_col(cols, 4),
                        model_used: str_col(cols, 5),
                        input_tokens: u64_col(cols, 6),
                        output_tokens: u64_col(cols, 7),
                        cache_read_tokens: u64_col(cols, 8),
                        cache_write_tokens: u64_col(cols, 9),
                        latency_ms: u64_col(cols, 10),
                        cost_usd: str_col(cols, 11),
                        created_at: str_col(cols, 12),
                    }));
                }
            }
        }
        Ok(None)
    }

    // ── Internals ───────────────────────────────────────────────────────

    async fn call_reducer(
        &self,
        reducer_name: &str,
        args: serde_json::Value,
    ) -> Result<(), String> {
        let url = format!(
            "{}/v1/database/{}/call/{}",
            self.host, self.database, reducer_name
        );

        let response = self
            .http
            .post(&url)
            .json(&args)
            .send()
            .await
            .map_err(|e| format!("SpacetimeDB call_reducer({}) failed: {}", reducer_name, e))?;

        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!(
                "Reducer '{}' returned {}: {}",
                reducer_name, status, body
            ))
        }
    }

    /// Execute a SQL query against the SpacetimeDB HTTP SQL endpoint.
    /// Returns the `rows` array from the first table in the response.
    async fn sql_query(
        &self,
        query: &str,
    ) -> Result<Vec<serde_json::Value>, String> {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);

        let response = self
            .http
            .post(&url)
            .body(query.to_string())
            .header("Content-Type", "text/plain")
            .send()
            .await
            .map_err(|e| format!("SpacetimeDB SQL query failed: {}", e))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("SQL query failed ({}): {}", status, body));
        }

        let body = response.text().await.unwrap_or_default();
        let parsed: serde_json::Value = serde_json::from_str(&body)
            .map_err(|e| format!("Failed to parse SQL response: {}", e))?;

        // SpacetimeDB SQL response: array of tables, each with a "rows" array
        let rows = parsed
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|table| table.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();

        Ok(rows)
    }
}

// ── Column extraction helpers ───────────────────────────────────────────

fn str_col(cols: &[serde_json::Value], idx: usize) -> String {
    cols.get(idx)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

fn u64_col(cols: &[serde_json::Value], idx: usize) -> u64 {
    cols.get(idx).and_then(|v| v.as_u64()).unwrap_or(0)
}

fn u32_col(cols: &[serde_json::Value], idx: usize) -> u32 {
    cols.get(idx).and_then(|v| v.as_u64()).unwrap_or(0) as u32
}
