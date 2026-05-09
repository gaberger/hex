//! `web_search` — Wave 2 grounding tool for external information.
//!
//! Lets personas ground their reasoning in current external info, not
//! just the local repo. Backend selection (priority order):
//!
//!   1. Tavily — set TAVILY_API_KEY (https://tavily.com, free tier
//!      1k queries/month, agent-optimised)
//!   2. Brave — set BRAVE_SEARCH_API_KEY (free tier 2k queries/month)
//!   3. DuckDuckGo HTML scrape (no key, fragile, may be rate-limited)
//!
//! The search result list is the same shape regardless of backend:
//!   { results: [{title, url, snippet}], backend, query, total }
//!
//! 5s timeout, max 10 results. Personas will typically follow up with
//! repo_grep / repo_read to pull more depth on whatever the search
//! surfaced.

use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

use super::{Tool, ToolResult};

const MAX_RESULTS_DEFAULT: usize = 5;
const MAX_RESULTS_HARD: usize = 10;
const QUERY_MAX_LEN: usize = 300;
const TIMEOUT_SECS: u64 = 8;

pub struct WebSearch;

#[async_trait]
impl Tool for WebSearch {
    fn name(&self) -> &'static str {
        "web_search"
    }
    fn description(&self) -> &'static str {
        "Search the public web for current information. Use this when \
         the operator's request needs up-to-date external info that the \
         hex repo doesn't contain (current best practices, library API \
         changes, ecosystem moves, recent papers). Returns title + URL + \
         snippet for each result. Follow up with repo_grep/repo_read to \
         ground in the codebase, OR cite the URL directly in an artifact."
    }
    fn input_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query, plain words. Max 300 chars. Be specific — vague queries waste the round trip."
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max results to return. Default 5, hard cap 10."
                }
            },
            "required": ["query"]
        })
    }
    async fn execute(&self, input: Value) -> ToolResult {
        let start = Instant::now();
        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() && q.len() <= QUERY_MAX_LEN => q.trim().to_string(),
            Some(q) => return ToolResult::err(
                format!("query length {} outside (0, {}]", q.len(), QUERY_MAX_LEN),
                start.elapsed().as_millis() as u64,
            ),
            None => return ToolResult::err("missing query", start.elapsed().as_millis() as u64),
        };
        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(MAX_RESULTS_DEFAULT as u64) as usize;
        let max_results = max_results.min(MAX_RESULTS_HARD);

        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(TIMEOUT_SECS))
            .user_agent("hex-nexus/web_search (Mozilla/5.0 compatible)")
            .build()
        {
            Ok(c) => c,
            Err(e) => return ToolResult::err(format!("http build: {}", e), start.elapsed().as_millis() as u64),
        };

        // Backend priority: Tavily, Brave, DDG.
        if let Ok(key) = std::env::var("TAVILY_API_KEY") {
            return search_tavily(&http, &key, &query, max_results, start).await;
        }
        if let Ok(key) = std::env::var("BRAVE_SEARCH_API_KEY") {
            return search_brave(&http, &key, &query, max_results, start).await;
        }
        search_duckduckgo(&http, &query, max_results, start).await
    }
}

async fn search_tavily(
    http: &reqwest::Client,
    key: &str,
    query: &str,
    max_results: usize,
    start: Instant,
) -> ToolResult {
    let body = json!({
        "api_key": key,
        "query": query,
        "max_results": max_results,
        "search_depth": "basic",
    });
    let resp = match http.post("https://api.tavily.com/search").json(&body).send().await {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("tavily http: {}", e), start.elapsed().as_millis() as u64),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return ToolResult::err(format!("tavily {}: {}", status, body.chars().take(200).collect::<String>()), start.elapsed().as_millis() as u64);
    }
    let v: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return ToolResult::err(format!("tavily json: {}", e), start.elapsed().as_millis() as u64),
    };
    let results: Vec<Value> = v
        .get("results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(max_results)
        .map(|r| json!({
            "title":   r.get("title").cloned().unwrap_or(Value::Null),
            "url":     r.get("url").cloned().unwrap_or(Value::Null),
            "snippet": r.get("content").cloned().unwrap_or(Value::Null),
        }))
        .collect();
    ToolResult::ok(
        json!({
            "backend": "tavily",
            "query":   query,
            "total":   results.len(),
            "results": results,
        }),
        start.elapsed().as_millis() as u64,
    )
}

async fn search_brave(
    http: &reqwest::Client,
    key: &str,
    query: &str,
    max_results: usize,
    start: Instant,
) -> ToolResult {
    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding(query),
        max_results,
    );
    let resp = match http
        .get(&url)
        .header("X-Subscription-Token", key)
        .header("Accept", "application/json")
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("brave http: {}", e), start.elapsed().as_millis() as u64),
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return ToolResult::err(format!("brave {}: {}", status, body.chars().take(200).collect::<String>()), start.elapsed().as_millis() as u64);
    }
    let v: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => return ToolResult::err(format!("brave json: {}", e), start.elapsed().as_millis() as u64),
    };
    let results: Vec<Value> = v
        .pointer("/web/results")
        .and_then(|r| r.as_array())
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .take(max_results)
        .map(|r| json!({
            "title":   r.get("title").cloned().unwrap_or(Value::Null),
            "url":     r.get("url").cloned().unwrap_or(Value::Null),
            "snippet": r.get("description").cloned().unwrap_or(Value::Null),
        }))
        .collect();
    ToolResult::ok(
        json!({
            "backend": "brave",
            "query":   query,
            "total":   results.len(),
            "results": results,
        }),
        start.elapsed().as_millis() as u64,
    )
}

async fn search_duckduckgo(
    http: &reqwest::Client,
    query: &str,
    max_results: usize,
    start: Instant,
) -> ToolResult {
    let url = format!("https://html.duckduckgo.com/html/?q={}", urlencoding(query));
    let resp = match http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => return ToolResult::err(format!("ddg http: {}", e), start.elapsed().as_millis() as u64),
    };
    if !resp.status().is_success() {
        return ToolResult::err(
            format!("ddg HTTP {} (set TAVILY_API_KEY or BRAVE_SEARCH_API_KEY for reliable backend)", resp.status()),
            start.elapsed().as_millis() as u64,
        );
    }
    let html = match resp.text().await {
        Ok(s) => s,
        Err(e) => return ToolResult::err(format!("ddg body: {}", e), start.elapsed().as_millis() as u64),
    };
    let results = parse_ddg_html(&html, max_results);
    if results.is_empty() {
        return ToolResult::err(
            "ddg returned 0 parseable results — likely rate-limited or HTML format changed; consider TAVILY_API_KEY",
            start.elapsed().as_millis() as u64,
        );
    }
    ToolResult::ok(
        json!({
            "backend": "duckduckgo",
            "query":   query,
            "total":   results.len(),
            "results": results,
        }),
        start.elapsed().as_millis() as u64,
    )
}

/// Cheap HTML extractor for DuckDuckGo's HTML interface. Looks for
/// `<a class="result__a" href="...">title</a>` and the matching
/// `<a class="result__snippet">snippet</a>` blocks. Brittle — DDG may
/// change their markup. The error message in search_duckduckgo points
/// the operator at the keyed alternatives.
fn parse_ddg_html(html: &str, max_results: usize) -> Vec<Value> {
    use regex::Regex;
    let title_re = Regex::new(
        r#"(?s)<a[^>]*class="[^"]*result__a[^"]*"[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#,
    )
    .unwrap();
    let snippet_re = Regex::new(
        r#"(?s)<a[^>]*class="[^"]*result__snippet[^"]*"[^>]*>(.*?)</a>"#,
    )
    .unwrap();
    let titles: Vec<(String, String)> = title_re
        .captures_iter(html)
        .take(max_results)
        .map(|c| (
            decode_url(c.get(1).map(|m| m.as_str()).unwrap_or("")),
            strip_tags(c.get(2).map(|m| m.as_str()).unwrap_or("")),
        ))
        .collect();
    let snippets: Vec<String> = snippet_re
        .captures_iter(html)
        .take(max_results)
        .map(|c| strip_tags(c.get(1).map(|m| m.as_str()).unwrap_or("")))
        .collect();
    titles
        .into_iter()
        .enumerate()
        .map(|(i, (url, title))| {
            json!({
                "title":   title,
                "url":     url,
                "snippet": snippets.get(i).cloned().unwrap_or_default(),
            })
        })
        .collect()
}

fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    // collapse whitespace
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode_url(u: &str) -> String {
    // DDG wraps results in /l/?uddg=<URL-encoded>; pull the inner URL out.
    if let Some(idx) = u.find("uddg=") {
        let tail = &u[idx + "uddg=".len()..];
        let end = tail.find('&').unwrap_or(tail.len());
        return percent_decode(&tail[..end]);
    }
    u.to_string()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(b) = u8::from_str_radix(hex, 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn urlencoding(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schema_requires_query() {
        let s = WebSearch.input_schema();
        let req = s.get("required").and_then(|v| v.as_array()).unwrap();
        assert!(req.iter().any(|v| v.as_str() == Some("query")));
    }
    #[test]
    fn url_encodes_spaces_to_plus() {
        assert_eq!(urlencoding("hello world"), "hello+world");
        assert_eq!(urlencoding("a&b=c"), "a%26b%3Dc");
    }
    #[test]
    fn percent_decode_round_trip() {
        assert_eq!(percent_decode("hello%20world"), "hello world");
        assert_eq!(percent_decode("a%26b"), "a&b");
    }
    #[test]
    fn strip_tags_basic() {
        assert_eq!(strip_tags("hello <b>world</b>"), "hello world");
        assert_eq!(strip_tags("a   b  c"), "a b c");
    }
}
