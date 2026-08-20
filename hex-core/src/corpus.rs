//! LoRA idiom-expert corpus types (ADR-2606161300, Phase 0 + Phase 1).
//!
//! hex encodes its own conventions into local tier models via small LoRA experts —
//! **for idiom injection only, never constraint enforcement** (ADR-2606161300 §1). A
//! `hex analyze` / behavioral-spec / best-of-N compile gate remains the sole arbiter
//! of correctness; an expert only raises the *first-draft* acceptance floor.
//!
//! This module is the pure-domain core (mirroring [`crate::resource_governor`]'s
//! posture): value types + a deterministic content hash, **zero I/O**. The corpus is
//! actually extracted in hex-nexus (`corpus_build`), which owns the filesystem; the
//! LoRA weights are trained by an external offline dev tool. Nothing here reaches the
//! network, the filesystem, or a model.
//!
//! The four design moves we borrow from DMoE (arXiv:2606.14243) are *decoupled
//! experts* (one [`KnowledgeUnit`] per expert, built in isolation) and a *content
//! hash* used as the corpus version stamp so a stale expert (trained on superseded
//! ADRs) is detectable — see [`content_hash`].

use serde::{Deserialize, Serialize};

/// A single expert's training scope: the source artifacts it learns its idiom from.
///
/// Decoupled, DMoE-style — building one unit never reads another's globs, which is
/// what keeps a `hex-testing` source from leaking into a `hex-boundaries` corpus
/// (spec `corpus-knowledge-unit-isolation`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnowledgeUnit {
    /// Expert name, e.g. `hex-boundaries`. Also the on-disk corpus directory name.
    pub expert: String,
    /// Glob patterns (repo-relative) selecting this expert's source artifacts.
    /// A source file is in-unit iff it matches at least one of these globs.
    pub source_globs: Vec<String>,
}

/// One instruction-tuning record, derived from exactly one source artifact.
///
/// PRAG-style augmentation (ADR-2606161300 §2): per source artifact we emit one
/// content-preserving paraphrase plus N Q/A pairs. Every record is traceable — it
/// **must** carry the `source_path` it came from so a human can audit the corpus
/// (spec `corpus-extraction-auditable-artifacts`). No answer-string leakage: records
/// teach style, never benchmark answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstructionPair {
    /// The instruction / question shown to the model.
    pub instruction: String,
    /// Optional additional input/context (may be empty).
    #[serde(default)]
    pub input: String,
    /// The target completion (the idiom-idiomatic answer).
    pub output: String,
    /// Repo-relative path of the source artifact this record was derived from.
    /// HARD: never empty — an untraceable record is a corpus bug.
    pub source_path: String,
    /// The corpus version this record was minted under (matches the manifest).
    pub corpus_version: String,
}

/// Auditable summary of a built corpus, written alongside `corpus.jsonl`.
///
/// The `content_hash` doubles as the corpus version and the staleness trigger: when a
/// source ADR/spec changes, a rebuild yields a different hash, and any adapter still
/// stamped with the old hash is flagged stale (spec `corpus-version-staleness-trigger`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusManifest {
    /// Expert this corpus belongs to.
    pub expert: String,
    /// Version stamp = [`content_hash`] of the records (also each record's stamp).
    pub corpus_version: String,
    /// The globs the corpus was built from (for audit + staleness re-resolution).
    pub source_globs: Vec<String>,
    /// Number of instruction records in `corpus.jsonl`.
    pub record_count: usize,
    /// Stable content hash over the records (== `corpus_version`).
    pub content_hash: String,
}

/// A registered LoRA adapter: a `(base_model, tier, expert)` binding plus the
/// corpus version it was trained on and its lifecycle flags.
///
/// `enabled` controls whether the serving path attaches it; `promoted` is set ONLY by
/// the external bench gate (ADR-2606161300 §5, spec `bench-gate-acceptance-lift-blocking`)
/// after a measured first-draft acceptance lift — never by training loss. Removing
/// every record restores the bare-base path with zero change to any correctness gate
/// (ADR-2606161300 §1 HARD invariant).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterRecord {
    /// Expert this adapter realizes, e.g. `hex-boundaries`.
    pub expert: String,
    /// Frozen base model the adapter rides on, e.g. `qwen2.5-coder:32b`.
    pub base_model: String,
    /// Tier the adapter serves (1, 2, or 25 for T2.5 — see tiered routing ADRs).
    pub tier: u8,
    /// Opaque reference to the trained artifact (path / registry ref / Ollama model).
    pub artifact_ref: String,
    /// Corpus version the adapter was trained on. If it differs from a fresh
    /// build's `content_hash`, the adapter is stale.
    pub corpus_version: String,
    /// Whether the serving path may attach this adapter.
    pub enabled: bool,
    /// Set true only by the bench gate on measured acceptance lift + no regression.
    #[serde(default)]
    pub promoted: bool,
}

/// Default expert → knowledge-unit mapping (ADR-2606161300 §2).
///
/// Used when `.hex/corpus/experts.toml` is absent. Globs are repo-relative and the
/// extractor (hex-nexus) is responsible for honoring `safePath` traversal protection
/// when resolving them. Phase 1 trains only `hex-boundaries`; the other three are
/// declared here so Phase 3 (`θ + ΣΔθᵢ` composition) needs no type change.
pub fn default_knowledge_units() -> Vec<KnowledgeUnit> {
    vec![
        KnowledgeUnit {
            expert: "hex-boundaries".to_string(),
            source_globs: vec![
                "docs/adrs/**/*hexagon*".to_string(),
                "docs/adrs/**/*boundar*".to_string(),
                "CLAUDE.md".to_string(),
                "docs/specs/**/*hexagon*".to_string(),
            ],
        },
        KnowledgeUnit {
            expert: "hex-rust-idiom".to_string(),
            source_globs: vec![
                "hex-core/src/**/*.rs".to_string(),
                "hex-nexus/src/ports/**/*.rs".to_string(),
            ],
        },
        KnowledgeUnit {
            expert: "hex-testing".to_string(),
            source_globs: vec![
                "docs/specs/**/*.json".to_string(),
                "**/tests/**/*.rs".to_string(),
            ],
        },
        KnowledgeUnit {
            expert: "hex-scaffold".to_string(),
            source_globs: vec!["hex-cli/assets/**".to_string()],
        },
    ]
}

/// Look up the default [`KnowledgeUnit`] for an expert name, if one is declared.
pub fn default_unit_for(expert: &str) -> Option<KnowledgeUnit> {
    default_knowledge_units()
        .into_iter()
        .find(|u| u.expert == expert)
}

/// Stable, dependency-free content hash over a corpus's records.
///
/// Used as the corpus version stamp and the staleness trigger (spec
/// `corpus-version-staleness-trigger`), so it MUST be deterministic across processes,
/// platforms, and Rust versions. `std`'s `DefaultHasher` (SipHash, randomly keyed /
/// version-unstable) is explicitly the wrong tool here; we use a fixed-seed FNV-1a so
/// the same records always hash identically and any change flips the digest.
///
/// The hash covers each record's semantic content (`instruction`, `input`, `output`,
/// `source_path`) in order. It deliberately excludes `corpus_version` itself — the
/// hash IS the version, so including it would be circular.
pub fn content_hash(records: &[InstructionPair]) -> String {
    // FNV-1a, 64-bit (offset basis / prime are the published constants).
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

    let mut hash = FNV_OFFSET;
    let mix = |bytes: &[u8], hash: &mut u64| {
        for &b in bytes {
            *hash ^= b as u64;
            *hash = hash.wrapping_mul(FNV_PRIME);
        }
        // Field separator so {"ab","c"} and {"a","bc"} don't collide.
        *hash ^= 0x1f;
        *hash = hash.wrapping_mul(FNV_PRIME);
    };

    for r in records {
        mix(r.instruction.as_bytes(), &mut hash);
        mix(r.input.as_bytes(), &mut hash);
        mix(r.output.as_bytes(), &mut hash);
        mix(r.source_path.as_bytes(), &mut hash);
        // Record separator.
        hash ^= 0xff;
        hash = hash.wrapping_mul(FNV_PRIME);
    }

    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(instruction: &str, output: &str, source: &str) -> InstructionPair {
        InstructionPair {
            instruction: instruction.to_string(),
            input: String::new(),
            output: output.to_string(),
            source_path: source.to_string(),
            corpus_version: String::new(),
        }
    }

    #[test]
    fn content_hash_is_stable_for_identical_records() {
        let a = vec![rec("q1", "a1", "docs/adrs/x.md")];
        let b = vec![rec("q1", "a1", "docs/adrs/x.md")];
        assert_eq!(content_hash(&a), content_hash(&b));
    }

    #[test]
    fn content_hash_changes_when_any_field_changes() {
        let base = vec![rec("q", "a", "p")];
        let h = content_hash(&base);
        assert_ne!(h, content_hash(&[rec("q!", "a", "p")]), "instruction");
        assert_ne!(h, content_hash(&[rec("q", "a!", "p")]), "output");
        assert_ne!(h, content_hash(&[rec("q", "a", "p!")]), "source_path");
    }

    #[test]
    fn content_hash_is_sensitive_to_field_boundaries() {
        // Separators must prevent {"ab","c"} from colliding with {"a","bc"}.
        let x = vec![rec("ab", "c", "p")];
        let y = vec![rec("a", "bc", "p")];
        assert_ne!(content_hash(&x), content_hash(&y));
    }

    #[test]
    fn content_hash_is_order_sensitive() {
        let r1 = rec("q1", "a1", "p1");
        let r2 = rec("q2", "a2", "p2");
        assert_ne!(
            content_hash(&[r1.clone(), r2.clone()]),
            content_hash(&[r2, r1])
        );
    }

    #[test]
    fn content_hash_is_fixed_width_hex() {
        let h = content_hash(&[rec("q", "a", "p")]);
        assert_eq!(h.len(), 16);
        assert!(h.chars().all(|c| c.is_ascii_hexdigit()));
        // Empty corpus hashes to the bare offset basis.
        assert_eq!(content_hash(&[]), "cbf29ce484222325");
    }

    #[test]
    fn default_units_cover_the_four_phase_experts() {
        let units = default_knowledge_units();
        let names: Vec<&str> = units.iter().map(|u| u.expert.as_str()).collect();
        assert!(names.contains(&"hex-boundaries"));
        assert!(names.contains(&"hex-rust-idiom"));
        assert!(names.contains(&"hex-testing"));
        assert!(names.contains(&"hex-scaffold"));
        // Every unit declares at least one glob.
        assert!(units.iter().all(|u| !u.source_globs.is_empty()));
    }

    #[test]
    fn default_unit_for_resolves_known_and_rejects_unknown() {
        assert_eq!(
            default_unit_for("hex-boundaries").map(|u| u.expert),
            Some("hex-boundaries".to_string())
        );
        assert!(default_unit_for("nonexistent-expert").is_none());
    }
}
