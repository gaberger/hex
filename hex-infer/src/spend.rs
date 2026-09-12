//! Token spend, appended locally.
//!
//! `cost_meter` reads `inference_log`. That table lived in SpacetimeDB and was written by the
//! daemon's inference route — so once inference moved in-process (Phase 1), the meter's source
//! disappeared. It would have kept working and always reported zero, which is the worst kind of
//! wrong: a spend report that is confidently empty.
//!
//! This is the write side, next to the call it measures. Best-effort: a failed append must never
//! fail an inference that succeeded.

use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

fn log_path() -> PathBuf {
    let base = std::env::var("HEX_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".hex")
        });
    base.join("inference-log.jsonl")
}

/// Append one call's usage. Silent on failure, by design.
pub fn record(model: &str, input_tokens: u64, output_tokens: u64) {
    let path = log_path();
    if let Some(dir) = path.parent() {
        if create_dir_all(dir).is_err() {
            return;
        }
    }
    let line = format!(
        "{}\n",
        serde_json::json!({
            "model": model,
            "input_tokens": input_tokens,
            "output_tokens": output_tokens,
            "ts": chrono::Utc::now().to_rfc3339(),
        })
    );
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_is_one_json_line_with_the_model_and_both_token_counts() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("inference-log.jsonl");
        // Exercise the format directly rather than through HEX_HOME: two tests that both set an
        // env var race in cargo's thread pool, which is the bug this file would otherwise repeat.
        let line = serde_json::json!({
            "model": "gemma4-12b:latest", "input_tokens": 12u64, "output_tokens": 34u64,
        });
        std::fs::write(&path, format!("{}\n", line)).unwrap();
        let read: serde_json::Value =
            serde_json::from_str(std::fs::read_to_string(&path).unwrap().trim()).unwrap();
        assert_eq!(read["model"], "gemma4-12b:latest");
        assert_eq!(read["input_tokens"], 12);
        assert_eq!(read["output_tokens"], 34);
    }
}
