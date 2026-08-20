//! Resource-governor consult for the do-loop (ADR-2606080915, Tier 0 wiring).
//!
//! Reads the real system (`/proc/meminfo` available RAM, `nvidia-smi` free VRAM) and
//! the candidate model's on-disk size (ollama `/api/tags`), then asks the pure
//! hex-core admission logic whether a LOCAL model's RAM-resident footprint (size minus
//! what fits in VRAM) fits alongside the compile-heavy job it drives. If not, the
//! best-of-N loop skips it and falls through to the `claude -p` frontier candidate
//! instead of OOMing — the exact failure that motivated the ADR.
//!
//! Fails OPEN: if anything can't be measured (no `/proc`, no GPU, ollama down), we
//! return [`AdmissionDecision::Admit`] so the governor never blocks a run it can't reason about.

use hex_core::resource_governor::{admit, parse_mem_available_mb, ram_footprint_after_offload_mb, AdmissionDecision};
use std::time::Duration;

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Working-set headroom (MB) reserved for the compile-heavy job the model drives
/// (cargo/tsc/go). Tonight's OOM was the model + this colliding. Override: `HEX_GOVERNOR_JOB_MB`.
fn job_headroom_mb() -> u64 {
    env_u64("HEX_GOVERNOR_JOB_MB", 4_000)
}

/// Safety margin (MB) never to consume. Override: `HEX_GOVERNOR_SAFETY_MB`.
fn safety_mb() -> u64 {
    env_u64("HEX_GOVERNOR_SAFETY_MB", 1_500)
}

/// Available system RAM in MB from `/proc/meminfo`; `None` if unreadable (non-Linux).
fn available_ram_mb() -> Option<u64> {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| parse_mem_available_mb(&s))
}

/// Free VRAM in MB across the first GPU via `nvidia-smi`; `0` if no GPU / tool absent
/// (no GPU → no offload → the whole model is RAM-resident, which is the correct
/// conservative reading).
async fn available_vram_mb() -> u64 {
    let out = tokio::process::Command::new("nvidia-smi")
        .args(["--query-gpu=memory.free", "--format=csv,noheader,nounits"])
        .output()
        .await;
    match out {
        Ok(o) if o.status.success() => String::from_utf8_lossy(&o.stdout)
            .lines()
            .next()
            .and_then(|l| l.trim().parse::<u64>().ok())
            .unwrap_or(0),
        _ => 0,
    }
}

/// Parse ollama `/api/tags` JSON → the named model's on-disk size in MB. Pure; tested.
pub fn parse_model_size_mb(tags_json: &str, model: &str) -> Option<u64> {
    let v: serde_json::Value = serde_json::from_str(tags_json).ok()?;
    for m in v.get("models")?.as_array()? {
        if m.get("name").and_then(|n| n.as_str()) == Some(model) {
            return Some(m.get("size")?.as_u64()? / (1024 * 1024));
        }
    }
    None
}

/// Query ollama for the model's on-disk size (MB); `None` if ollama is unreachable or
/// the model is unknown.
async fn model_size_mb(model: &str) -> Option<u64> {
    let host = std::env::var("OLLAMA_HOST").unwrap_or_else(|_| "127.0.0.1:11434".to_string());
    let host = if host.starts_with("http") { host } else { format!("http://{host}") };
    let http = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .ok()?;
    let body = http
        .get(format!("{host}/api/tags"))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    parse_model_size_mb(&body, model)
}

/// Decide whether to run `model` locally or route to the frontier fallback, from the
/// real system state. Fails OPEN to [`AdmissionDecision::Admit`] when unmeasurable.
pub async fn check_local_model(model: &str) -> AdmissionDecision {
    let (Some(avail_ram), Some(size)) = (available_ram_mb(), model_size_mb(model).await) else {
        return AdmissionDecision::Admit; // can't measure → don't interfere
    };
    let ram_footprint = ram_footprint_after_offload_mb(size, available_vram_mb().await);
    admit(ram_footprint, job_headroom_mb(), avail_ram, safety_mb())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAGS: &str = r#"{"models":[
        {"name":"devstral-small-2:24b","size":15032385536},
        {"name":"hf.co/unsloth/Qwen3-Coder-Next-GGUF:Q2_K","size":31138512896}
    ]}"#;

    #[test]
    fn parses_named_model_size_to_mb() {
        // 15032385536 bytes / 1024^2 = 14336 MB.
        assert_eq!(parse_model_size_mb(TAGS, "devstral-small-2:24b"), Some(14_336));
    }

    #[test]
    fn unknown_model_is_none() {
        assert_eq!(parse_model_size_mb(TAGS, "nope:latest"), None);
    }

    #[test]
    fn malformed_json_is_none() {
        assert_eq!(parse_model_size_mb("not json", "x"), None);
    }
}
