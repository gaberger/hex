use async_trait::async_trait;
use reqwest::Client;
use std::time::Duration;

use crate::domain::context::ContextVariables;
use crate::ports::live_context::ILiveContextPort;

pub struct LiveContextAdapter {
    client: Client,
    nexus_url: String,
}

impl LiveContextAdapter {
    pub fn new(nexus_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            client,
            nexus_url: nexus_url.into(),
        }
    }

    async fn fetch_analyze(&self) -> (Option<u8>, Option<Vec<String>>) {
        let url = format!("{}/api/analyze?path=.", self.nexus_url);
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return (None, None),
        };
        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return (None, None),
        };
        let score = json.get("score").and_then(|v| v.as_u64()).map(|v| v.min(255) as u8);
        let violations = json
            .get("violations")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str().map(String::from))
                    .collect::<Vec<_>>()
            });
        (score, violations)
    }

    async fn fetch_adrs(&self, task: &str) -> Option<Vec<String>> {
        let url = format!(
            "{}/api/adrs/search?q={}",
            self.nexus_url,
            urlencoding_simple(task)
        );
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return None,
        };
        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return None,
        };
        json.get("results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let id = item.get("id")?.as_str()?;
                        let title = item.get("title")?.as_str()?;
                        Some(format!("{id}: {title}"))
                    })
                    .collect::<Vec<_>>()
            })
    }

    async fn fetch_summary(&self, files: &[String]) -> Option<String> {
        if files.is_empty() {
            return None;
        }
        let joined = files.join(",");
        let url = format!(
            "{}/api/summarize?files={}",
            self.nexus_url,
            urlencoding_simple(&joined)
        );
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return None,
        };
        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return None,
        };
        json.get("summary")
            .and_then(|v| v.as_str())
            .map(String::from)
    }

    async fn fetch_diff(&self) -> Option<String> {
        let url = format!("{}/api/git/diff", self.nexus_url);
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return None,
        };
        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return None,
        };
        json.get("diff")
            .and_then(|v| v.as_str())
            .map(|s| s.chars().take(2000).collect())
    }

    async fn fetch_memory(&self, task: &str) -> Option<String> {
        let url = format!(
            "{}/api/hexflo/memory/search?q={}",
            self.nexus_url,
            urlencoding_simple(task)
        );
        let resp = match self.client.get(&url).send().await {
            Ok(r) => r,
            Err(_) => return None,
        };
        let json: serde_json::Value = match resp.json().await {
            Ok(v) => v,
            Err(_) => return None,
        };
        let result = json
            .get("results")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|item| {
                        let key = item.get("key")?.as_str()?;
                        let value = item.get("value")?.as_str()?;
                        Some(format!("{key}: {value}"))
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            });
        match result {
            Some(s) if s.is_empty() => None,
            other => other,
        }
    }
}

#[async_trait]
impl ILiveContextPort for LiveContextAdapter {
    async fn enrich(
        &self,
        vars: &mut ContextVariables,
        task: &str,
        files: &[String],
    ) -> Result<(), crate::ports::live_context::LiveContextError> {
        let (analyze_fut, adrs_fut, summary_fut, diff_fut, memory_fut) = (
            self.fetch_analyze(),
            self.fetch_adrs(task),
            self.fetch_summary(files),
            self.fetch_diff(),
            self.fetch_memory(task),
        );

        let ((architecture_score, arch_violations), relevant_adrs, ast_summary, recent_changes, hexflo_memory) =
            tokio::join!(analyze_fut, adrs_fut, summary_fut, diff_fut, memory_fut);

        vars.architecture_score = architecture_score;
        vars.arch_violations = arch_violations;
        vars.relevant_adrs = relevant_adrs;
        vars.ast_summary = ast_summary;
        vars.recent_changes = recent_changes;
        vars.hexflo_memory = hexflo_memory;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::get, response::Json as AxumJson};
    use serde_json::json;

    async fn start_test_server(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        format!("http://127.0.0.1:{}", addr.port())
    }

    fn all_ok_router() -> Router {
        Router::new()
            .route(
                "/api/analyze",
                get(|| async {
                    AxumJson(json!({ "score": 85, "violations": ["missing port abstraction"] }))
                }),
            )
            .route(
                "/api/adrs/search",
                get(|| async {
                    AxumJson(json!({ "results": [{ "id": "ADR-001", "title": "Use hexagonal arch" }] }))
                }),
            )
            .route(
                "/api/summarize",
                get(|| async {
                    AxumJson(json!({ "summary": "Auth module: handles login and token refresh" }))
                }),
            )
            .route(
                "/api/git/diff",
                get(|| async { AxumJson(json!({ "diff": "+added line\n-removed line" })) }),
            )
            .route(
                "/api/hexflo/memory/search",
                get(|| async {
                    AxumJson(json!({ "results": [{ "key": "auth-pattern", "value": "use JWT" }] }))
                }),
            )
    }

    fn all_404_router() -> Router {
        use axum::http::StatusCode;
        Router::new()
            .route("/api/analyze", get(|| async { StatusCode::NOT_FOUND }))
            .route("/api/adrs/search", get(|| async { StatusCode::NOT_FOUND }))
            .route("/api/summarize", get(|| async { StatusCode::NOT_FOUND }))
            .route("/api/git/diff", get(|| async { StatusCode::NOT_FOUND }))
            .route(
                "/api/hexflo/memory/search",
                get(|| async { StatusCode::NOT_FOUND }),
            )
    }

    fn partial_router() -> Router {
        use axum::http::StatusCode;
        Router::new()
            .route(
                "/api/analyze",
                get(|| async {
                    AxumJson(json!({ "score": 85, "violations": ["missing port abstraction"] }))
                }),
            )
            .route(
                "/api/adrs/search",
                get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
            )
            .route(
                "/api/summarize",
                get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
            )
            .route(
                "/api/git/diff",
                get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
            )
            .route(
                "/api/hexflo/memory/search",
                get(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
            )
    }

    #[tokio::test]
    async fn test_enrich_all_fields_populated() {
        let url = start_test_server(all_ok_router()).await;
        let adapter = LiveContextAdapter::new(&url);
        let mut vars = ContextVariables::default();
        let result = adapter
            .enrich(&mut vars, "implement auth", &["src/auth.rs".to_string()])
            .await;

        assert!(result.is_ok());
        assert_eq!(vars.architecture_score, Some(85));
        assert_eq!(vars.arch_violations.as_ref().unwrap().len(), 1);
        assert!(vars.relevant_adrs.is_some());
        assert!(vars.ast_summary.is_some());
        assert!(vars.recent_changes.is_some());
        assert!(vars.hexflo_memory.is_some());
    }

    #[tokio::test]
    async fn test_enrich_degrades_gracefully_on_404() {
        let url = start_test_server(all_404_router()).await;
        let adapter = LiveContextAdapter::new(&url);
        let mut vars = ContextVariables::default();
        let result = adapter
            .enrich(&mut vars, "implement auth", &["src/auth.rs".to_string()])
            .await;

        assert!(result.is_ok());
        assert!(vars.architecture_score.is_none());
        assert!(vars.arch_violations.is_none());
        assert!(vars.relevant_adrs.is_none());
        assert!(vars.ast_summary.is_none());
        assert!(vars.recent_changes.is_none());
        assert!(vars.hexflo_memory.is_none());
    }

    #[tokio::test]
    async fn test_enrich_partial_failure() {
        let url = start_test_server(partial_router()).await;
        let adapter = LiveContextAdapter::new(&url);
        let mut vars = ContextVariables::default();
        let result = adapter
            .enrich(&mut vars, "implement auth", &["src/auth.rs".to_string()])
            .await;

        assert!(result.is_ok());
        assert_eq!(vars.architecture_score, Some(85));
        assert!(vars.arch_violations.is_some());
        assert!(vars.relevant_adrs.is_none());
        assert!(vars.ast_summary.is_none());
        assert!(vars.recent_changes.is_none());
        assert!(vars.hexflo_memory.is_none());
    }
}

/// Minimal percent-encoding for query string values (encodes spaces and special chars).
fn urlencoding_simple(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for b in input.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
                out.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
            }
        }
    }
    out
}
