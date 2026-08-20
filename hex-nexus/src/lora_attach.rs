//! LoRA adapter attachment on the serving path (ADR-2606161300, Phase 1, step 6).
//!
//! When the tier router resolves a request to a local base model, this module checks
//! whether an enabled idiom adapter is registered for it and, if so, returns the name
//! of a *derived* Ollama model (`FROM <base>` + `ADAPTER <gguf>`) to call instead. The
//! adapter attaches to the model weights only — it is created once per
//! `(base, artifact_ref)` and cached by Ollama, so the long ReAct loop keeps reusing
//! the same prefix KV-cache (DMoE's final-FFN-attachment efficiency result; the LoRA
//! does not invalidate the cache the way a dynamic-RAG prompt rewrite would).
//!
//! **It uses no new router** (DMoE's "training-free routing"): selection rides the
//! existing tier resolution that already produced `base_model`.
//!
//! **It touches no gate.** The only thing this changes is *which weights generate the
//! draft*. `hex analyze`, behavioral specs, and the best-of-N compile gate run exactly
//! as they would on the bare base (ADR-2606161300 §1, spec
//! `enforcement-stays-external-negative`). On ANY failure — LoRA disabled, no adapter,
//! Ollama unreachable, create error — it returns `None` and the caller serves the bare
//! base, never an error (spec `inference-path-adapter-attachment` fallback).

use crate::lora_registry::AdapterStore;

/// Derive the Ollama model name for a base + expert. Ollama tags use `:`/`/` so we
/// fold those to `_` to keep the derived name a single valid tag.
pub fn derived_model_name(base_model: &str, expert: &str) -> String {
    let sanitized: String = base_model
        .chars()
        .map(|c| if c == ':' || c == '/' || c == ' ' { '_' } else { c })
        .collect();
    format!("hexlora-{expert}-{sanitized}")
}

/// Resolve the model to actually serve for a `(provider, base_model)` the router chose.
///
/// Returns `Some(derived_model)` only when: LoRA experts are enabled, the provider is a
/// local Ollama, an enabled adapter is registered for the base, and the derived model
/// is present (or was just created) in Ollama. Otherwise `None` → caller serves the
/// bare base. Never returns an error — attachment is best-effort by contract.
pub async fn resolve_serving_model(
    provider: &str,
    base_model: &str,
    ollama_url: &str,
) -> Option<String> {
    // Kill switch / project setting (HEX_LORA_DISABLED, inference.lora.enabled).
    if !crate::state_config::resolve_lora_experts_enabled() {
        return None;
    }
    // Adapters only attach to local Ollama bases in Phase 1.
    if provider != "ollama" || ollama_url.is_empty() {
        return None;
    }

    // Cheap, no-network exit when nothing is registered for this base (the default).
    let adapters = AdapterStore::from_env().enabled_for_base(base_model);
    let adapter = adapters.first()?;
    if adapters.len() > 1 {
        // Phase 3 composes θ + ΣΔθᵢ; Phase 1 serves the first registered expert and
        // logs the rest so a multi-expert config isn't silently narrowed.
        tracing::info!(
            base = %base_model,
            chosen = %adapter.expert,
            others = adapters.len() - 1,
            "multiple LoRA experts registered for base — Phase 1 serves the first (composition is Phase 3)"
        );
    }

    let derived = derived_model_name(base_model, &adapter.expert);
    match ensure_ollama_model(ollama_url, &derived, base_model, &adapter.artifact_ref).await {
        Ok(()) => {
            tracing::info!(
                base = %base_model,
                expert = %adapter.expert,
                derived = %derived,
                "serving base+LoRA idiom adapter (idiom prior only — gates unchanged)"
            );
            Some(derived)
        }
        Err(e) => {
            // Best-effort: fall back to the bare base, never fail the request.
            tracing::warn!(
                base = %base_model,
                expert = %adapter.expert,
                error = %e,
                "LoRA adapter attach failed — serving bare base"
            );
            None
        }
    }
}

/// Ensure a derived Ollama model exists, creating it once from the frozen base + the
/// LoRA GGUF. Idempotent: if the model already exists we skip the (re)create so cached
/// weights + KV-cache are reused.
///
/// Ollama ≥0.6 dropped the inline-Modelfile create; file-based models must reference an
/// uploaded blob by sha256 digest. So: (1) sha256 the GGUF, (2) upload it as a blob,
/// (3) `create {from, adapters:{name: "sha256:<digest>"}}`.
async fn ensure_ollama_model(
    ollama_url: &str,
    derived: &str,
    base_model: &str,
    artifact_ref: &str,
) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(180))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let base = ollama_url.trim_end_matches('/');

    if ollama_has_model(&client, ollama_url, derived).await {
        return Ok(());
    }

    // (1) Read the adapter GGUF and compute its sha256 digest.
    let bytes = tokio::fs::read(artifact_ref)
        .await
        .map_err(|e| format!("read adapter '{artifact_ref}': {e}"))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();

    // (2) Upload the blob (201 created / 200 already-present are both fine).
    let blob_url = format!("{base}/api/blobs/sha256:{digest}");
    let blob_resp = client
        .post(&blob_url)
        .body(bytes)
        .send()
        .await
        .map_err(|e| format!("ollama blob upload: {e}"))?;
    if !blob_resp.status().is_success() {
        return Err(format!("ollama blob upload returned {}", blob_resp.status()));
    }

    // (3) Create FROM the frozen base with the LoRA adapter blob (FFN per the ADR).
    let create_url = format!("{base}/api/create");
    let resp = client
        .post(&create_url)
        .json(&serde_json::json!({
            "model": derived,
            "from": base_model,
            "adapters": { "adapter.gguf": format!("sha256:{digest}") },
            "stream": false,
        }))
        .send()
        .await
        .map_err(|e| format!("ollama create request: {e}"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let code = resp.status();
        let body = resp.text().await.unwrap_or_default();
        Err(format!("ollama create returned {code}: {body}"))
    }
}

/// Best-effort check whether Ollama already serves a model named `name`.
async fn ollama_has_model(client: &reqwest::Client, ollama_url: &str, name: &str) -> bool {
    let url = format!("{}/api/tags", ollama_url.trim_end_matches('/'));
    let Ok(resp) = client.get(&url).send().await else {
        return false;
    };
    let Ok(body) = resp.json::<serde_json::Value>().await else {
        return false;
    };
    body.get("models")
        .and_then(|m| m.as_array())
        .map(|arr| {
            arr.iter().any(|m| {
                m.get("name")
                    .and_then(|n| n.as_str())
                    .map(|n| n == name || n.starts_with(&format!("{name}:")))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_name_is_a_valid_single_tag() {
        let n = derived_model_name("qwen2.5-coder:32b", "hex-boundaries");
        assert_eq!(n, "hexlora-hex-boundaries-qwen2.5-coder_32b");
        assert!(!n.contains(':') || n.matches(':').count() == 0);
        assert!(!n.contains('/'));
    }

    #[tokio::test]
    async fn non_ollama_provider_never_attaches() {
        // Cloud/openrouter providers never get a LoRA adapter — bare base.
        let got = resolve_serving_model("openrouter", "google/gemini-2.0", "").await;
        assert!(got.is_none());
    }

    #[tokio::test]
    async fn no_registered_adapter_serves_bare_base() {
        // With nothing registered for this base, attachment is a no-op (no network).
        // (resolve_lora_experts_enabled defaults true; the empty registry short-circuits.)
        std::env::remove_var("HEX_LORA_DISABLED");
        let got = resolve_serving_model("ollama", "no-such-base-model:1b", "http://127.0.0.1:1").await;
        assert!(got.is_none(), "no adapter → bare base");
    }

    #[tokio::test]
    async fn disabled_kill_switch_serves_bare_base() {
        std::env::set_var("HEX_LORA_DISABLED", "1");
        let got = resolve_serving_model("ollama", "qwen2.5-coder:32b", "http://127.0.0.1:1").await;
        std::env::remove_var("HEX_LORA_DISABLED");
        assert!(got.is_none(), "kill switch → bare base");
    }
}
