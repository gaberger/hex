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
    async fn enrich(&self, task: &str, files: &[String]) -> ContextVariables {
        let (analyze_fut, adrs_fut, summary_fut, diff_fut, memory_fut) = (
            self.fetch_analyze(),
            self.fetch_adrs(task),
            self.fetch_summary(files),
            self.fetch_diff(),
            self.fetch_memory(task),
        );

        let ((architecture_score, arch_violations), relevant_adrs, ast_summary, recent_changes, hexflo_memory) =
            tokio::join!(analyze_fut, adrs_fut, summary_fut, diff_fut, memory_fut);

        ContextVariables {
            architecture_score,
            arch_violations,
            relevant_adrs,
            ast_summary,
            recent_changes,
            hexflo_memory,
            spec_content: None,
        }
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
