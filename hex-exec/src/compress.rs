//! Context-window compression for the ReAct loop (ADR-2606071XXX).
//!
//! A multi-step tool-use loop accumulates tool observations (grep hits, file
//! reads, cargo errors) in the message history. Left unbounded they blow the
//! model's context window and bloat every subsequent call. This module keeps the
//! transcript small *deterministically* (no model call) via two mechanisms:
//!
//!   1. per-observation head+tail cap — each tool_result is clamped to a budget,
//!      keeping the informative ends and eliding the middle;
//!   2. rolling window — only the last K observation turns are kept in full;
//!      older ones collapse to a one-line gist.
//!
//! When even the mechanical pass leaves the transcript over a token budget, the
//! loop (not this module) does ONE cheap-model summarization of the elided
//! region — the hybrid strategy. This module stays pure + unit-tested; the LLM
//! step lives in `direct_react`. Recursive decomposition over large observations
//! (RLM, arXiv:2512.24601) is the frontier beyond this compaction baseline.

use serde_json::{json, Value};

/// Knobs for mechanical compression.
#[derive(Debug, Clone, Copy)]
pub struct CompressOpts {
    /// Bytes kept from the head of each observation.
    pub obs_head: usize,
    /// Bytes kept from the tail of each observation.
    pub obs_tail: usize,
    /// How many of the most-recent observation turns to keep verbatim (capped).
    /// Older observation turns collapse to a one-line gist.
    pub keep_recent: usize,
}

impl Default for CompressOpts {
    fn default() -> Self {
        Self { obs_head: 1024, obs_tail: 1024, keep_recent: 4 }
    }
}

/// Clamp a string to `head` + `tail` bytes (on char boundaries), inserting an
/// elision marker. Returns the clamped string. No-op when already small enough.
pub fn cap_str(s: &str, head: usize, tail: usize) -> String {
    if s.len() <= head + tail + 32 {
        return s.to_string();
    }
    let head_end = floor_char_boundary(s, head);
    let tail_start = ceil_char_boundary(s, s.len().saturating_sub(tail));
    let elided = tail_start.saturating_sub(head_end);
    format!("{}\n…[elided {} bytes]…\n{}", &s[..head_end], elided, &s[tail_start..])
}

/// Rough token estimate (chars / 4) over the whole message array — enough to
/// decide when the LLM-overflow summarization should fire.
pub fn estimate_tokens(messages: &[Value]) -> usize {
    let mut chars = 0usize;
    for m in messages {
        chars += content_chars(m.get("content").unwrap_or(&Value::Null));
    }
    chars / 4
}

fn content_chars(content: &Value) -> usize {
    match content {
        Value::String(s) => s.len(),
        Value::Array(blocks) => blocks
            .iter()
            .map(|b| {
                // tool_use input, text, or tool_result content string.
                b.get("content").and_then(|v| v.as_str()).map(|s| s.len()).unwrap_or(0)
                    + b.get("text").and_then(|v| v.as_str()).map(|s| s.len()).unwrap_or(0)
                    + b.get("input").map(|v| v.to_string().len()).unwrap_or(0)
            })
            .sum(),
        _ => 0,
    }
}

/// Returns true if a message is a user turn carrying tool_result blocks (an
/// "observation" turn produced by the loop after dispatching tools).
fn is_observation_turn(m: &Value) -> bool {
    m.get("role").and_then(|v| v.as_str()) == Some("user")
        && m.get("content")
            .and_then(|c| c.as_array())
            .map(|blocks| blocks.iter().any(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_result")))
            .unwrap_or(false)
}

/// Mechanically compress the transcript: cap every observation to head+tail, and
/// collapse all but the last `keep_recent` observation turns to a one-line gist.
/// The first message (the task/seed) and assistant reasoning turns pass through.
pub fn compress_messages(messages: &[Value], opts: &CompressOpts) -> Vec<Value> {
    // Index the observation turns so we can keep only the most recent K full.
    let obs_indices: Vec<usize> =
        messages.iter().enumerate().filter(|(_, m)| is_observation_turn(m)).map(|(i, _)| i).collect();
    let keep_from = obs_indices.len().saturating_sub(opts.keep_recent);
    let recent_obs: std::collections::HashSet<usize> =
        obs_indices.iter().skip(keep_from).copied().collect();

    messages
        .iter()
        .enumerate()
        .map(|(i, m)| {
            if !is_observation_turn(m) {
                return m.clone();
            }
            let blocks = m.get("content").and_then(|c| c.as_array()).cloned().unwrap_or_default();
            if recent_obs.contains(&i) {
                // Recent: keep, but cap each observation's payload.
                let capped: Vec<Value> = blocks
                    .into_iter()
                    .map(|b| cap_tool_result(b, opts))
                    .collect();
                json!({ "role": "user", "content": capped })
            } else {
                // Old: collapse every tool_result to a one-line gist.
                let gists: Vec<Value> = blocks
                    .into_iter()
                    .map(|b| gist_tool_result(b))
                    .collect();
                json!({ "role": "user", "content": gists })
            }
        })
        .collect()
}

fn cap_tool_result(mut b: Value, opts: &CompressOpts) -> Value {
    if b.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
        return b;
    }
    if let Some(s) = b.get("content").and_then(|v| v.as_str()) {
        let capped = cap_str(s, opts.obs_head, opts.obs_tail);
        if let Some(obj) = b.as_object_mut() {
            obj.insert("content".to_string(), json!(capped));
        }
    }
    b
}

fn gist_tool_result(b: Value) -> Value {
    if b.get("type").and_then(|v| v.as_str()) != Some("tool_result") {
        return b;
    }
    let id = b.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("");
    let bytes = b.get("content").and_then(|v| v.as_str()).map(|s| s.len()).unwrap_or(0);
    let is_err = b.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
    json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": format!("[earlier observation elided — {} bytes{}]", bytes, if is_err { ", was error" } else { "" }),
        "is_error": is_err,
    })
}

// `str::floor_char_boundary`/`ceil_char_boundary` are unstable; provide local ones.
fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char_boundary(s: &str, mut i: usize) -> usize {
    if i >= s.len() {
        return s.len();
    }
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_str_keeps_ends_and_elides_middle() {
        let s = "A".repeat(5000);
        let capped = cap_str(&s, 100, 100);
        assert!(capped.len() < s.len());
        assert!(capped.starts_with(&"A".repeat(100)));
        assert!(capped.contains("elided"));
        assert!(capped.ends_with(&"A".repeat(100)));
    }

    #[test]
    fn cap_str_noop_when_small() {
        let s = "short";
        assert_eq!(cap_str(s, 1024, 1024), "short");
    }

    #[test]
    fn cap_str_respects_char_boundaries() {
        // Multibyte chars at the cut points must not panic / split a char.
        let s = format!("{}{}", "é".repeat(2000), "ü".repeat(2000));
        let capped = cap_str(&s, 101, 101); // 101 is mid-2-byte-char
        assert!(capped.contains("elided"));
    }

    #[test]
    fn estimate_tokens_counts_blocks() {
        let msgs = vec![
            json!({"role":"user","content":"hello world"}),
            json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"x","content":"A".repeat(400)}]}),
        ];
        // (11 + 400) / 4 ≈ 102
        let t = estimate_tokens(&msgs);
        assert!(t >= 100 && t <= 105, "got {}", t);
    }

    #[test]
    fn rolling_window_gists_old_observations() {
        // 6 observation turns, keep_recent=2 → first 4 become gists, last 2 capped.
        let big = "X".repeat(5000);
        let mut msgs = vec![json!({"role":"user","content":"task seed"})];
        for i in 0..6 {
            msgs.push(json!({"role":"assistant","content":[{"type":"text","text":format!("step {}",i)}]}));
            msgs.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":format!("t{}",i),"content":big.clone()}]}));
        }
        let opts = CompressOpts { obs_head: 200, obs_tail: 200, keep_recent: 2 };
        let out = compress_messages(&msgs, &opts);

        let obs: Vec<&Value> = out.iter().filter(|m| is_observation_turn(m)).collect();
        assert_eq!(obs.len(), 6);
        // First 4 are gists (short), last 2 are capped (head+tail+marker, still < big).
        for m in &obs[..4] {
            let c = m["content"][0]["content"].as_str().unwrap();
            assert!(c.contains("earlier observation elided"), "old should be gist: {}", c);
        }
        for m in &obs[4..] {
            let c = m["content"][0]["content"].as_str().unwrap();
            assert!(c.contains("elided") && c.len() < big.len(), "recent should be capped");
            assert!(!c.contains("earlier observation"), "recent should not be gisted");
        }
        // Seed + assistant turns pass through untouched.
        assert_eq!(out[0]["content"].as_str().unwrap(), "task seed");
    }

    #[test]
    fn compression_reduces_token_estimate() {
        let big = "Y".repeat(8000);
        let mut msgs = vec![json!({"role":"user","content":"seed"})];
        for i in 0..5 {
            msgs.push(json!({"role":"user","content":[{"type":"tool_result","tool_use_id":format!("t{}",i),"content":big.clone()}]}));
        }
        let before = estimate_tokens(&msgs);
        let out = compress_messages(&msgs, &CompressOpts::default());
        let after = estimate_tokens(&out);
        assert!(after < before / 2, "compression should roughly halve+: {} -> {}", before, after);
    }
}
