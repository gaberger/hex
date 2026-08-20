//! LoRA adapter registry (ADR-2606161300, Phase 1).
//!
//! A small, daemon-managed store binding trained LoRA adapters to a
//! `(base_model, tier, expert)` tuple plus the corpus version they were trained on.
//! It is a *sibling* record set: it never touches the inference-endpoint records, so
//! removing every adapter restores the bare-base path with zero change to any
//! correctness gate (ADR-2606161300 §1 HARD invariant).
//!
//! **Storage.** Phase 1 persists to `.hex/inference/lora-adapters.json` (atomic
//! write), managed entirely inside hex-nexus — this is runtime state owned by the
//! daemon, not a shell script (CLAUDE.md no-runtime-scripts rule). The STDB-backed
//! table alongside `inference_provider` (ADR-025) is the production follow-up
//! (Phase 3/5); the [`AdapterStore`] surface below is the seam that swap rides on, so
//! callers never learn the backend.
//!
//! The registry stores only *metadata + an artifact reference*; the actual GGUF
//! adapter weights are produced offline by `scripts/train-lora.sh` and referenced by
//! `artifact_ref`.

use std::path::{Path, PathBuf};

use hex_core::corpus::AdapterRecord;

/// Stable, human-readable id for an adapter record: `<expert>:<base>:t<tier>`.
///
/// One adapter per `(expert, base_model, tier)` — registering the same tuple twice
/// updates in place rather than duplicating.
pub fn record_id(r: &AdapterRecord) -> String {
    format!("{}:{}:t{}", r.expert, r.base_model, r.tier)
}

/// Path to the registry file under a repo root.
fn registry_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".hex/inference/lora-adapters.json")
}

/// File-backed adapter registry. Cheap to construct; all state lives on disk so it is
/// consistent across daemon restarts and multiple nexus instances reading the file.
pub struct AdapterStore {
    repo_root: PathBuf,
}

impl AdapterStore {
    /// Open the store rooted at `repo_root` (the directory containing `.hex/`).
    pub fn new(repo_root: impl Into<PathBuf>) -> Self {
        Self { repo_root: repo_root.into() }
    }

    /// Open the store at the conventional repo root (`HEX_REPO_ROOT` / cwd).
    pub fn from_env() -> Self {
        Self::new(crate::corpus_build::resolve_repo_root())
    }

    /// All registered adapters (empty when the registry has never been written).
    pub fn list(&self) -> Vec<AdapterRecord> {
        let path = registry_path(&self.repo_root);
        let Ok(text) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        serde_json::from_str::<Vec<AdapterRecord>>(&text).unwrap_or_else(|e| {
            tracing::warn!(error = %e, path = %path.display(), "corrupt lora registry — treating as empty");
            Vec::new()
        })
    }

    /// Register (or update in place) an adapter. Idempotent on [`record_id`].
    pub fn register(&self, record: AdapterRecord) -> Result<(), String> {
        let id = record_id(&record);
        let mut all = self.list();
        if let Some(existing) = all.iter_mut().find(|r| record_id(r) == id) {
            *existing = record;
        } else {
            all.push(record);
        }
        self.save(&all)
    }

    /// Remove an adapter by id. Returns true if one was removed.
    ///
    /// Removal restores the bare-base serving path for that tuple with zero change to
    /// any correctness gate (ADR-2606161300 §1).
    pub fn remove(&self, id: &str) -> Result<bool, String> {
        let mut all = self.list();
        let before = all.len();
        all.retain(|r| record_id(r) != id);
        let removed = all.len() != before;
        if removed {
            self.save(&all)?;
        }
        Ok(removed)
    }

    /// Enable or disable an adapter by id. Returns true if one was updated.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<bool, String> {
        self.mutate(id, |r| r.enabled = enabled)
    }

    /// Mark an adapter promoted/un-promoted by id (bench gate only). Returns true if updated.
    pub fn set_promoted(&self, id: &str, promoted: bool) -> Result<bool, String> {
        self.mutate(id, |r| r.promoted = promoted)
    }

    /// Find the enabled adapters that apply to a `(base_model, tier)` the serving path
    /// resolved. Multiple experts may apply → caller composes them (`θ + ΣΔθᵢ`).
    /// Disabled records are never returned (spec `inference-path-adapter-attachment`).
    pub fn enabled_for(&self, base_model: &str, tier: u8) -> Vec<AdapterRecord> {
        self.list()
            .into_iter()
            .filter(|r| r.enabled && r.base_model == base_model && r.tier == tier)
            .collect()
    }

    /// Like [`enabled_for`] but matches on base model alone — used by the serving path
    /// where the request carries a model name but not an explicit tier (a local model
    /// maps to a single tier in practice). Disabled records are never returned.
    pub fn enabled_for_base(&self, base_model: &str) -> Vec<AdapterRecord> {
        self.list()
            .into_iter()
            .filter(|r| r.enabled && r.base_model == base_model)
            .collect()
    }

    fn mutate(&self, id: &str, f: impl FnOnce(&mut AdapterRecord)) -> Result<bool, String> {
        let mut all = self.list();
        let Some(rec) = all.iter_mut().find(|r| record_id(r) == id) else {
            return Ok(false);
        };
        f(rec);
        self.save(&all)?;
        Ok(true)
    }

    /// Atomically persist the full record set (temp file + rename).
    fn save(&self, records: &[AdapterRecord]) -> Result<(), String> {
        let path = registry_path(&self.repo_root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create registry dir: {e}"))?;
        }
        let json =
            serde_json::to_string_pretty(records).map_err(|e| format!("serialize registry: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("write registry tmp: {e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("commit registry: {e}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(expert: &str, base: &str, tier: u8) -> AdapterRecord {
        AdapterRecord {
            expert: expert.to_string(),
            base_model: base.to_string(),
            tier,
            artifact_ref: "/tmp/adapter.gguf".to_string(),
            corpus_version: "abc123".to_string(),
            enabled: true,
            promoted: false,
        }
    }

    fn tmp_root() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let id = N.fetch_add(1, Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("hex-lora-reg-{}-{}", std::process::id(), id));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn register_list_remove_roundtrip() {
        let root = tmp_root();
        let store = AdapterStore::new(&root);
        assert!(store.list().is_empty());

        store.register(rec("hex-boundaries", "qwen2.5-coder:32b", 2)).unwrap();
        let all = store.list();
        assert_eq!(all.len(), 1);
        let id = record_id(&all[0]);

        // Survives a fresh store instance (persisted to disk).
        let store2 = AdapterStore::new(&root);
        assert_eq!(store2.list().len(), 1);

        assert!(store2.remove(&id).unwrap());
        assert!(store2.list().is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn register_is_idempotent_on_tuple() {
        let root = tmp_root();
        let store = AdapterStore::new(&root);
        store.register(rec("hex-boundaries", "qwen2.5-coder:32b", 2)).unwrap();
        let mut updated = rec("hex-boundaries", "qwen2.5-coder:32b", 2);
        updated.artifact_ref = "/new/path.gguf".to_string();
        store.register(updated).unwrap();
        let all = store.list();
        assert_eq!(all.len(), 1, "same tuple updates in place");
        assert_eq!(all[0].artifact_ref, "/new/path.gguf");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn enabled_for_filters_by_tuple_and_enabled() {
        let root = tmp_root();
        let store = AdapterStore::new(&root);
        store.register(rec("hex-boundaries", "qwen2.5-coder:32b", 2)).unwrap();
        store.register(rec("hex-rust-idiom", "qwen2.5-coder:32b", 2)).unwrap();
        let mut other_tier = rec("hex-testing", "qwen2.5-coder:32b", 1);
        other_tier.enabled = true;
        store.register(other_tier).unwrap();

        let matches = store.enabled_for("qwen2.5-coder:32b", 2);
        assert_eq!(matches.len(), 2, "two experts apply to (qwen2.5-coder, t2)");

        // Disable one → no longer returned.
        let id = record_id(&matches[0]);
        store.set_enabled(&id, false).unwrap();
        assert_eq!(store.enabled_for("qwen2.5-coder:32b", 2).len(), 1);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn set_promoted_updates_flag() {
        let root = tmp_root();
        let store = AdapterStore::new(&root);
        store.register(rec("hex-boundaries", "qwen2.5-coder:32b", 2)).unwrap();
        let id = record_id(&store.list()[0]);
        assert!(store.set_promoted(&id, true).unwrap());
        assert!(store.list()[0].promoted);
        assert!(!store.set_promoted("nonexistent:x:t9", true).unwrap());
        std::fs::remove_dir_all(&root).ok();
    }
}
