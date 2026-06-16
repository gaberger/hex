//! LoRA idiom-expert corpus extraction (ADR-2606161300, Phase 0).
//!
//! hex-nexus is the filesystem bridge (WASM modules can't touch the FS), so corpus
//! extraction lives here: read an expert's declared source artifacts from disk, apply
//! PRAG-style augmentation, and write **human-auditable** instruction records to
//! `.hex/corpus/<expert>/`. The pure types + the version hash live in
//! [`hex_core::corpus`]; this module is the I/O adapter around them.
//!
//! Two HARD properties enforced here (behavioral specs in
//! `docs/specs/hex-lora-idiom-phase01.json`):
//! 1. **Every record carries a `source_path`** — no untraceable records
//!    (`corpus-extraction-auditable-artifacts`).
//! 2. **Knowledge-unit isolation** — a source outside the expert's globs never enters
//!    its corpus, and building one expert never touches another's directory
//!    (`corpus-knowledge-unit-isolation`).
//!
//! Augmentation routes through the existing tiered-inference path (a T1/T2 call) when
//! an inference port is wired. When it isn't (offline build, unit tests), a
//! deterministic source-derived fallback keeps every record traceable — it never
//! fabricates content the source doesn't contain.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use hex_core::corpus::{content_hash, default_unit_for, CorpusManifest, InstructionPair, KnowledgeUnit};
use hex_core::domain::messages::{ContentBlock, Message};
use hex_core::ports::inference::{IInferencePort, InferenceRequest, Priority};

/// Per-build configuration (resolved by the caller from `state_config`).
pub struct CorpusBuildConfig {
    /// Repo root the globs resolve against.
    pub repo_root: PathBuf,
    /// Number of Q/A pairs to mint per source artifact (PRAG recipe).
    pub qa_count: usize,
    /// When true, compute the manifest but write nothing (`--dry-run`).
    pub dry_run: bool,
    /// Tier model used for augmentation (e.g. the T1 model). Ignored when no
    /// inference port is supplied.
    pub augment_model: String,
}

/// Cap on bytes read per source artifact — bounds prompt size + corpus growth.
const MAX_SOURCE_BYTES: usize = 8_000;
/// Cap on source files per expert — a runaway glob shouldn't mint a giant corpus.
const MAX_SOURCE_FILES: usize = 200;

/// Resolve the repo root the same way the rest of hex-nexus does.
pub fn resolve_repo_root() -> PathBuf {
    std::env::var("HEX_REPO_ROOT")
        .or_else(|_| std::env::var("CLAUDE_PROJECT_DIR"))
        .or_else(|_| std::env::var("HEX_PROJECT_DIR"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
}

/// Resolve the [`KnowledgeUnit`] for an expert: a `.hex/corpus/experts.toml` override
/// if present, else the embedded default mapping ([`hex_core::corpus::default_unit_for`]).
pub fn resolve_unit(repo_root: &Path, expert: &str) -> Result<KnowledgeUnit, String> {
    let overrides_path = repo_root.join(".hex/corpus/experts.toml");
    if let Ok(text) = std::fs::read_to_string(&overrides_path) {
        if let Some(unit) = parse_experts_toml(&text, expert) {
            return Ok(unit);
        }
    }
    default_unit_for(expert).ok_or_else(|| {
        format!("unknown expert '{expert}' (no experts.toml entry and no default mapping)")
    })
}

/// Parse `experts.toml` of the shape `[experts.<name>] globs = ["..", ".."]` and
/// return the unit for `expert` if declared. Returns `None` on any parse miss so the
/// caller falls back to the embedded default.
fn parse_experts_toml(text: &str, expert: &str) -> Option<KnowledgeUnit> {
    let value: toml::Value = toml::from_str(text).ok()?;
    let globs = value
        .get("experts")?
        .get(expert)?
        .get("globs")?
        .as_array()?
        .iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect::<Vec<_>>();
    if globs.is_empty() {
        return None;
    }
    Some(KnowledgeUnit { expert: expert.to_string(), source_globs: globs })
}

/// Translate a repo-relative glob into an anchored regex.
///
/// Supports `**` (any depth, including zero, when written `**/`), `*` (any run of
/// non-separator chars), and `?` (one non-separator char). All other regex
/// metacharacters are escaped so the glob means exactly what it says.
fn glob_to_regex(glob: &str) -> regex::Regex {
    let bytes: Vec<char> = glob.chars().collect();
    let mut out = String::from("^");
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            '*' => {
                let double = i + 1 < bytes.len() && bytes[i + 1] == '*';
                if double {
                    // `**/` collapses any number of leading path segments (incl. none).
                    if i + 2 < bytes.len() && bytes[i + 2] == '/' {
                        out.push_str("(?:.*/)?");
                        i += 3;
                    } else {
                        out.push_str(".*");
                        i += 2;
                    }
                } else {
                    out.push_str("[^/]*");
                    i += 1;
                }
            }
            '?' => {
                out.push_str("[^/]");
                i += 1;
            }
            // Escape regex specials; '/' is a literal separator.
            '.' | '+' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$' | '\\' => {
                out.push('\\');
                out.push(c);
                i += 1;
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    out.push('$');
    // Globs are author-controlled (defaults or experts.toml); a malformed one is a
    // config bug — fall back to a never-matching pattern rather than panic.
    regex::Regex::new(&out).unwrap_or_else(|_| regex::Regex::new("$.^").unwrap())
}

/// True if `rel_path` matches at least one of the expert's globs.
fn matches_unit(rel_path: &str, compiled: &[regex::Regex]) -> bool {
    compiled.iter().any(|re| re.is_match(rel_path))
}

/// Directory names never descended into during source collection.
fn is_skipped_dir(name: &str) -> bool {
    matches!(
        name,
        "target" | "node_modules" | ".git" | ".hex" | "dist" | "build" | ".cache" | "out"
    )
}

/// Collect `(repo_relative_path, content)` for every in-unit source file.
///
/// Walks `repo_root`, skips build/VCS dirs and non-UTF-8 files, keeps only paths that
/// match the unit's globs (isolation), and caps file count + per-file bytes.
fn collect_sources(repo_root: &Path, unit: &KnowledgeUnit) -> Vec<(String, String)> {
    let compiled: Vec<regex::Regex> = unit.source_globs.iter().map(|g| glob_to_regex(g)).collect();
    let mut out: Vec<(String, String)> = Vec::new();

    for entry in walkdir::WalkDir::new(repo_root)
        .into_iter()
        .filter_entry(|e| {
            !(e.file_type().is_dir()
                && e.file_name().to_str().is_some_and(is_skipped_dir))
        })
    {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = match entry.path().strip_prefix(repo_root) {
            Ok(p) => p.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        if !matches_unit(&rel, &compiled) {
            continue;
        }
        // Only UTF-8 text; binary assets are skipped (not all hex-cli/assets are text).
        let content = match std::fs::read_to_string(entry.path()) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let truncated: String = content.chars().take(MAX_SOURCE_BYTES).collect();
        out.push((rel, truncated));
        if out.len() >= MAX_SOURCE_FILES {
            tracing::warn!(
                expert = %unit.expert,
                cap = MAX_SOURCE_FILES,
                "corpus source cap reached — remaining matches skipped"
            );
            break;
        }
    }
    // Stable order → stable content hash across runs.
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// Augment one source artifact into instruction records.
///
/// With an inference port: ask the tier model for a content-preserving paraphrase +
/// `qa_count` Q/A pairs, parsed from JSON. On any inference/parse failure — or with no
/// port at all — fall back to a deterministic source-chunk split so the build always
/// yields traceable records and never blocks on a model.
async fn augment(
    inference: Option<&Arc<dyn IInferencePort>>,
    cfg: &CorpusBuildConfig,
    rel_path: &str,
    content: &str,
) -> Vec<(String, String, String)> {
    if let Some(port) = inference {
        if let Some(pairs) = augment_via_model(port, cfg, rel_path, content).await {
            if !pairs.is_empty() {
                return pairs;
            }
        }
        tracing::warn!(source = %rel_path, "augmentation model unavailable/unparseable — using source-derived fallback");
    }
    fallback_chunks(cfg.qa_count, rel_path, content)
}

/// Model-driven augmentation. Returns `None` on any failure so the caller falls back.
async fn augment_via_model(
    port: &Arc<dyn IInferencePort>,
    cfg: &CorpusBuildConfig,
    rel_path: &str,
    content: &str,
) -> Option<Vec<(String, String, String)>> {
    let prompt = format!(
        "You are building a STYLE training corpus for hex's coding conventions.\n\
         Source artifact: {rel_path}\n\
         --- SOURCE START ---\n{content}\n--- SOURCE END ---\n\n\
         Return ONLY JSON of the form:\n\
         {{\"paraphrase\": \"<restate the source's guidance in your own words, \
         preserving meaning, inventing NOTHING not present above>\", \
         \"qa\": [{{\"q\": \"<question about the convention>\", \"a\": \"<answer grounded \
         strictly in the source>\"}}]}}\n\
         Produce exactly {} q/a pairs. Do not leak any answer string not derivable \
         from the source.",
        cfg.qa_count
    );

    let req = InferenceRequest {
        model: cfg.augment_model.clone(),
        system_prompt: "Output strict JSON only. Never invent facts absent from the provided source.".to_string(),
        messages: vec![Message::user(&prompt)],
        tools: vec![],
        max_tokens: 2048,
        temperature: 0.3,
        thinking_budget: None,
        cache_control: false,
        priority: Priority::Low,
        grammar: None,
    };

    let resp = port.complete(req).await.ok()?;
    let text: String = resp
        .content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();

    parse_augmentation_json(&text, rel_path)
}

/// Extract `{paraphrase, qa:[{q,a}]}` from a model response (tolerant of prose around
/// the JSON object). Returns `None` if no usable object is found.
fn parse_augmentation_json(text: &str, rel_path: &str) -> Option<Vec<(String, String, String)>> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    if end <= start {
        return None;
    }
    let value: serde_json::Value = serde_json::from_str(&text[start..=end]).ok()?;

    let mut pairs = Vec::new();
    if let Some(p) = value.get("paraphrase").and_then(|v| v.as_str()) {
        if !p.trim().is_empty() {
            pairs.push((
                format!("Restate the hex convention from `{rel_path}` in your own words."),
                String::new(),
                p.trim().to_string(),
            ));
        }
    }
    if let Some(qa) = value.get("qa").and_then(|v| v.as_array()) {
        for item in qa {
            let q = item.get("q").and_then(|v| v.as_str()).unwrap_or("").trim();
            let a = item.get("a").and_then(|v| v.as_str()).unwrap_or("").trim();
            if !q.is_empty() && !a.is_empty() {
                pairs.push((q.to_string(), String::new(), a.to_string()));
            }
        }
    }
    if pairs.is_empty() {
        None
    } else {
        Some(pairs)
    }
}

/// Deterministic source-derived fallback: split the artifact into up to `qa_count + 1`
/// contiguous chunks and emit each verbatim. Source-grounded, traceable, model-free.
fn fallback_chunks(qa_count: usize, rel_path: &str, content: &str) -> Vec<(String, String, String)> {
    let trimmed = content.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let n = (qa_count + 1).max(1);
    let chars: Vec<char> = trimmed.chars().collect();
    let chunk = chars.len().div_ceil(n).max(1);
    chars
        .chunks(chunk)
        .map(|c| {
            let body: String = c.iter().collect();
            (
                format!("Reproduce the hex idiom from `{rel_path}`."),
                String::new(),
                body,
            )
        })
        .collect()
}

/// Build (and optionally write) the corpus for one expert.
///
/// Returns the [`CorpusManifest`]. On `dry_run`, computes the manifest without writing
/// anything to disk (spec `corpus-extraction-auditable-artifacts`).
pub async fn build_corpus(
    expert: &str,
    cfg: &CorpusBuildConfig,
    inference: Option<&Arc<dyn IInferencePort>>,
) -> Result<CorpusManifest, String> {
    let unit = resolve_unit(&cfg.repo_root, expert)?;
    let sources = collect_sources(&cfg.repo_root, &unit);

    // Mint records (corpus_version stamped after the hash is known — the hash
    // deliberately excludes that field, so post-stamping doesn't change it).
    let mut records: Vec<InstructionPair> = Vec::new();
    for (rel_path, content) in &sources {
        for (instruction, input, output) in augment(inference, cfg, rel_path, content).await {
            records.push(InstructionPair {
                instruction,
                input,
                output,
                source_path: rel_path.clone(),
                corpus_version: String::new(),
            });
        }
    }

    // HARD invariant: every record is traceable.
    debug_assert!(
        records.iter().all(|r| !r.source_path.is_empty()),
        "corpus record without source_path"
    );

    let version = content_hash(&records);
    for r in &mut records {
        r.corpus_version = version.clone();
    }

    let manifest = CorpusManifest {
        expert: unit.expert.clone(),
        corpus_version: version.clone(),
        source_globs: unit.source_globs.clone(),
        record_count: records.len(),
        content_hash: version,
    };

    if !cfg.dry_run {
        write_corpus(&cfg.repo_root, &unit.expert, &records, &manifest)?;
    }

    Ok(manifest)
}

/// Write `corpus.jsonl` (newline-delimited [`InstructionPair`]) + `manifest.json`.
fn write_corpus(
    repo_root: &Path,
    expert: &str,
    records: &[InstructionPair],
    manifest: &CorpusManifest,
) -> Result<(), String> {
    let dir = repo_root.join(".hex/corpus").join(expert);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create corpus dir: {e}"))?;

    let mut jsonl = String::new();
    for r in records {
        let line = serde_json::to_string(r).map_err(|e| format!("serialize record: {e}"))?;
        jsonl.push_str(&line);
        jsonl.push('\n');
    }
    std::fs::write(dir.join("corpus.jsonl"), jsonl).map_err(|e| format!("write corpus.jsonl: {e}"))?;

    let manifest_json =
        serde_json::to_string_pretty(manifest).map_err(|e| format!("serialize manifest: {e}"))?;
    std::fs::write(dir.join("manifest.json"), manifest_json)
        .map_err(|e| format!("write manifest.json: {e}"))?;

    Ok(())
}

/// Freshly recompute an expert's manifest content hash WITHOUT writing — used by the
/// adapter-registry staleness check (spec `corpus-version-staleness-trigger`). Returns
/// the current `content_hash` for the expert's sources, or an error if unresolvable.
pub async fn current_corpus_hash(
    expert: &str,
    cfg: &CorpusBuildConfig,
) -> Result<String, String> {
    let dry = CorpusBuildConfig {
        repo_root: cfg.repo_root.clone(),
        qa_count: cfg.qa_count,
        dry_run: true,
        augment_model: cfg.augment_model.clone(),
    };
    // No inference: the fallback is deterministic, so the hash is a stable function of
    // the SOURCE content — exactly what staleness should track (idiom drift comes from
    // the source changing, not from model sampling noise).
    Ok(build_corpus(expert, &dry, None).await?.content_hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(root: &Path, rel: &str, body: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn tmp() -> PathBuf {
        // Process-unique temp dir without Math.random (use pid + a static counter).
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let id = N.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("hex-corpus-test-{}-{}", std::process::id(), id));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn cfg_for(root: &Path, dry_run: bool) -> CorpusBuildConfig {
        CorpusBuildConfig {
            repo_root: root.to_path_buf(),
            qa_count: 2,
            dry_run,
            augment_model: "test-model".to_string(),
        }
    }

    #[test]
    fn glob_matches_expected_paths() {
        let re = glob_to_regex("docs/adrs/**/*hexagon*");
        assert!(re.is_match("docs/adrs/ADR-1-hexagon-rules.md"));
        assert!(re.is_match("docs/adrs/sub/foo-hexagon.md"));
        assert!(!re.is_match("docs/specs/hexagon.json"));

        let lit = glob_to_regex("CLAUDE.md");
        assert!(lit.is_match("CLAUDE.md"));
        assert!(!lit.is_match("xCLAUDExmd"));

        let rs = glob_to_regex("hex-core/src/**/*.rs");
        assert!(rs.is_match("hex-core/src/lib.rs"));
        assert!(rs.is_match("hex-core/src/ports/inference.rs"));
        assert!(!rs.is_match("hex-core/src/lib.txt"));

        let assets = glob_to_regex("hex-cli/assets/**");
        assert!(assets.is_match("hex-cli/assets/a/b/c.yaml"));
        assert!(!assets.is_match("hex-cli/src/main.rs"));
    }

    #[tokio::test]
    async fn every_record_has_source_path_and_version() {
        let root = tmp();
        write(&root, "docs/adrs/ADR-1-hexagon.md", "An adapter MUST NOT import another adapter.");
        write(&root, "CLAUDE.md", "All relative imports use .js extensions.");

        let cfg = cfg_for(&root, false);
        let m = build_corpus("hex-boundaries", &cfg, None).await.unwrap();
        assert!(m.record_count > 0);
        assert_eq!(m.content_hash, m.corpus_version);

        // Read back corpus.jsonl: every record traceable + stamped.
        let jsonl = std::fs::read_to_string(root.join(".hex/corpus/hex-boundaries/corpus.jsonl")).unwrap();
        for line in jsonl.lines().filter(|l| !l.trim().is_empty()) {
            let r: InstructionPair = serde_json::from_str(line).unwrap();
            assert!(!r.source_path.is_empty(), "record missing source_path");
            assert_eq!(r.corpus_version, m.corpus_version);
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn knowledge_unit_isolation_excludes_other_experts_sources() {
        let root = tmp();
        // In-unit for hex-boundaries:
        write(&root, "docs/adrs/ADR-2-hexagon.md", "composition-root is the only file importing adapters.");
        // hex-testing-only artifact — must NOT appear in hex-boundaries corpus:
        write(&root, "docs/specs/some-spec.json", "{\"behavioral\": \"spec\"}");

        let cfg = cfg_for(&root, false);
        build_corpus("hex-boundaries", &cfg, None).await.unwrap();

        let jsonl = std::fs::read_to_string(root.join(".hex/corpus/hex-boundaries/corpus.jsonl")).unwrap();
        for line in jsonl.lines().filter(|l| !l.trim().is_empty()) {
            let r: InstructionPair = serde_json::from_str(line).unwrap();
            assert!(
                !r.source_path.starts_with("docs/specs/"),
                "hex-testing source leaked into hex-boundaries corpus: {}",
                r.source_path
            );
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn dry_run_writes_nothing() {
        let root = tmp();
        write(&root, "CLAUDE.md", "Adapters never import other adapters.");

        let cfg = cfg_for(&root, true);
        let m = build_corpus("hex-boundaries", &cfg, None).await.unwrap();
        assert!(m.record_count > 0);
        assert!(
            !root.join(".hex/corpus/hex-boundaries").exists(),
            "dry-run must not write the corpus directory"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn unknown_expert_errors() {
        let root = tmp();
        let cfg = cfg_for(&root, true);
        assert!(build_corpus("nope-not-an-expert", &cfg, None).await.is_err());
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn staleness_hash_tracks_source_changes() {
        let root = tmp();
        write(&root, "CLAUDE.md", "version one of the rules");
        let cfg = cfg_for(&root, true);
        let h1 = current_corpus_hash("hex-boundaries", &cfg).await.unwrap();

        write(&root, "CLAUDE.md", "version TWO of the rules — changed");
        let h2 = current_corpus_hash("hex-boundaries", &cfg).await.unwrap();
        assert_ne!(h1, h2, "hash must change when source changes");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn experts_toml_override_parses() {
        let toml = r#"
[experts.custom-expert]
globs = ["docs/foo/**/*.md", "bar.txt"]
"#;
        let unit = parse_experts_toml(toml, "custom-expert").unwrap();
        assert_eq!(unit.expert, "custom-expert");
        assert_eq!(unit.source_globs.len(), 2);
        assert!(parse_experts_toml(toml, "absent").is_none());
    }
}
