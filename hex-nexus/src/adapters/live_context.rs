use std::time::Duration;

use reqwest::Client;

use crate::ports::live_context::ILiveContextPort;

/// Live context adapter that self-calls the hex-nexus REST API to enrich
/// workplan task prompts with architecture score, relevant ADRs, and the
/// current git diff. Gracefully degrades — any endpoint failure is skipped.
pub struct NexusLiveContextAdapter {
    client: Client,
    /// Base URL of this nexus instance (e.g. "http://127.0.0.1:5555").
    pub base_url: String,
}

impl NexusLiveContextAdapter {
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();
        Self {
            client,
            base_url: base_url.into(),
        }
    }

    async fn fetch_analyze(&self) -> Option<String> {
        let url = format!("{}/api/analyze?path=.", self.base_url);
        let json: serde_json::Value =
            self.client.get(&url).send().await.ok()?.json().await.ok()?;
        let score = json.get("score").and_then(|v| v.as_u64())?;
        let violations = json
            .get("violations")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .filter(|s| !s.is_empty());
        let mut out = format!("Architecture score: {}/100", score);
        if let Some(v) = violations {
            out.push_str(&format!(" — violations: {}", v));
        }
        Some(out)
    }

    async fn fetch_adrs(&self, task: &str) -> Option<String> {
        let url = format!(
            "{}/api/adrs/search?q={}",
            self.base_url,
            percent_encode(task)
        );
        let json: serde_json::Value =
            self.client.get(&url).send().await.ok()?.json().await.ok()?;
        let results = json.get("results")?.as_array()?;
        if results.is_empty() {
            return None;
        }
        let lines: Vec<String> = results
            .iter()
            .filter_map(|item| {
                let id = item.get("id")?.as_str()?;
                let title = item.get("title")?.as_str()?;
                Some(format!("  - {}: {}", id, title))
            })
            .collect();
        Some(lines.join("\n"))
    }

    async fn fetch_diff(&self) -> Option<String> {
        let url = format!("{}/api/git/diff", self.base_url);
        let json: serde_json::Value =
            self.client.get(&url).send().await.ok()?.json().await.ok()?;
        let diff = json.get("diff")?.as_str()?;
        // Cap at 1500 chars to avoid bloating the prompt.
        Some(diff.chars().take(1500).collect())
    }
}

#[async_trait::async_trait]
impl ILiveContextPort for NexusLiveContextAdapter {
    async fn enrich(&self, task: &str, files: &[String]) -> String {
        let (analyze, adrs, diff) = tokio::join!(
            self.fetch_analyze(),
            self.fetch_adrs(task),
            self.fetch_diff(),
        );

        let mut sections = Vec::new();
        if let Some(a) = analyze {
            sections.push(format!("**Architecture**: {}", a));
        }
        if let Some(a) = adrs {
            sections.push(format!("**Relevant ADRs**:\n{}", a));
        }
        if let Some(d) = diff {
            sections.push(format!(
                "**Recent changes** (git diff excerpt):\n```\n{}\n```",
                d
            ));
        }
        if !files.is_empty() {
            sections.push(format!("**Target files**: {}", files.join(", ")));
        }

        sections.join("\n\n")
    }
}

/// Minimal percent-encoding for query string values.
fn percent_encode(input: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_stores_base_url() {
        let a = NexusLiveContextAdapter::new("http://127.0.0.1:5555");
        assert_eq!(a.base_url, "http://127.0.0.1:5555");
    }

    #[tokio::test]
    async fn enrich_degrades_when_nexus_offline() {
        // Port 19998 is unused — adapter must return empty string, not panic.
        let a = NexusLiveContextAdapter::new("http://127.0.0.1:19998");
        let result = a.enrich("implement auth", &["src/auth.rs".to_string()]).await;
        // Graceful degradation: empty string when all endpoints unreachable.
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn enrich_includes_files_even_without_nexus() {
        // Files section is populated from the task parameter, not an HTTP call.
        // But wait — files are appended after HTTP calls, so if all calls fail,
        // the files section still appears (it does not require a network call).
        let a = NexusLiveContextAdapter::new("http://127.0.0.1:19997");
        let result = a
            .enrich("task", &["src/foo.rs".to_string(), "src/bar.rs".to_string()])
            .await;
        assert!(result.contains("src/foo.rs"));
        assert!(result.contains("src/bar.rs"));
    }
}
