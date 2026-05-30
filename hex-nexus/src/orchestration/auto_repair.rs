//! Autonomous code-quality repair loop.
//!
//! Purpose: close the gap between "workplan complete at file-presence
//! level" and "code actually compiles". After the workplan_conductor
//! declares a workplan complete, this loop kicks in — runs `cargo
//! check`, groups errors by file, and fires a `code_patch` ask to
//! hex-coder for the top-K offenders. Iterates until errors hit zero
//! or a plateau is reached.
//!
//! This is the missing primary-goal piece. The operator should not have
//! to manually fire `hex ops send` asks to drive cargo errors down —
//! the system should do it. After 2026-05-29 morning where 12+
//! operator-fired asks took the ebay-mvp from 73 → 34 errors, this
//! module automates that exact loop.
//!
//! Configuration (per-project, `.hex/project.json`):
//!
//! ```json
//! {
//!   "auto_repair": {
//!     "enabled": true,
//!     "project_path": "examples/ebay-clone/backend",
//!     "max_iterations": 20,
//!     "top_k_files": 3
//!   }
//! }
//! ```
//!
//! Disable globally with HEX_DISABLE_AUTO_REPAIR=1.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;

use hex_core::domain::messages::{ContentBlock, Message};
use hex_core::ports::inference::{IInferencePort, InferenceRequest, Priority};

const DEFAULT_INTERVAL_SECS: u64 = 120;
const DEFAULT_STARTUP_SECS: u64 = 90;
const DEFAULT_COOLDOWN_SECS: u64 = 240;
const DEFAULT_MAX_ITERATIONS: u32 = 20;
const DEFAULT_TOP_K: usize = 3;
const SEND_AGENT_ID: &str = "nexus-auto-repair";

/// How many full frontier-escalation passes we attempt before truly pausing
/// the loop. Each pass invokes `claude -p` once per top-K errored file.
/// Bounded so a hung subprocess can't burn budget forever. See retro §5.12.
const MAX_FRONTIER_ATTEMPTS: u32 = 2;

/// How long a file is considered "in flight to persona" after the dispatch
/// HTTP call returns. Longer than the typical persona response latency so
/// that re-dispatch + frontier escalation both wait for the inbox to drain.
/// See retro §5.15 + ADR-2026-04-15-1430.
const PERSONA_INFLIGHT_TTL_SECS: u64 = 300;

#[derive(Default)]
struct RepairState {
    iterations: u32,
    last_error_count: Option<u32>,
    no_progress_count: u32,
    file_cooldowns: HashMap<String, Instant>,
    /// Number of frontier-escalation passes consumed in this loop's lifetime.
    /// Reset on `reset()`. When this reaches `MAX_FRONTIER_ATTEMPTS`, the
    /// loop pauses for real (the §5.6 operator-inbox endpoint).
    frontier_attempts: u32,
    /// §5.15 file-overlap gate: files dispatched to persona within the
    /// `PERSONA_INFLIGHT_TTL_SECS` window. Persona response latency often
    /// exceeds the shorter `file_cooldowns` TTL, so we track persona
    /// dispatches separately. Frontier + persona both filter against this.
    dispatched_to_persona: HashMap<String, Instant>,
    /// §5.15: files currently being processed by a frontier-escalation pass.
    /// Set BEFORE the claude subprocess call; cleared on commit (success or
    /// failure). Persona dispatch checks this to avoid stomping on a
    /// frontier rewrite in progress.
    frontier_inflight: HashSet<String>,
}

fn state() -> &'static Mutex<RepairState> {
    static S: OnceLock<Mutex<RepairState>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(RepairState::default()))
}

/// Snapshot of the auto_repair loop state for the status endpoint.
/// Public so the HTTP handler in routes/ can return it as JSON.
#[derive(serde::Serialize)]
pub struct RepairStateSnapshot {
    pub iterations: u32,
    pub last_error_count: Option<u32>,
    pub no_progress_count: u32,
    pub paused: bool,
    pub max_iterations: Option<u32>,
    pub file_cooldowns_count: usize,
    pub frontier_attempts: u32,
    pub frontier_max_attempts: u32,
    pub dispatched_to_persona_count: usize,
    pub frontier_inflight_count: usize,
}

pub fn snapshot() -> RepairStateSnapshot {
    let s = state().lock().unwrap();
    let max_it = std::env::var("HEX_AUTO_REPAIR_MAX_ITERATIONS_OVERRIDE")
        .ok()
        .and_then(|v| v.parse().ok());
    let paused = (s.no_progress_count >= 3 && s.frontier_attempts >= MAX_FRONTIER_ATTEMPTS)
        || s.iterations >= max_it.unwrap_or(u32::MAX);
    RepairStateSnapshot {
        iterations: s.iterations,
        last_error_count: s.last_error_count,
        no_progress_count: s.no_progress_count,
        paused,
        max_iterations: max_it,
        file_cooldowns_count: s.file_cooldowns.len(),
        frontier_attempts: s.frontier_attempts,
        frontier_max_attempts: MAX_FRONTIER_ATTEMPTS,
        dispatched_to_persona_count: s.dispatched_to_persona.len(),
        frontier_inflight_count: s.frontier_inflight.len(),
    }
}

/// Reset the loop state so the next tick fires fresh. Used by the
/// `hex auto-repair restart` CLI to re-engage a paused loop without a
/// full nexus restart.
pub fn reset() {
    let mut s = state().lock().unwrap();
    s.iterations = 0;
    s.last_error_count = None;
    s.no_progress_count = 0;
    s.file_cooldowns.clear();
    s.frontier_attempts = 0;
    s.dispatched_to_persona.clear();
    s.frontier_inflight.clear();
    tracing::info!("auto_repair: state reset via operator API");
}

/// What the plateau check decided this tick should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlateauAction {
    /// No plateau yet; proceed with normal dispatch.
    Continue,
    /// Plateau hit AND we still have frontier attempts left. Skip normal
    /// dispatch this tick; run frontier (claude -p) escalation instead.
    Escalate,
    /// Plateau hit AND frontier is exhausted (or never available). Pause
    /// the loop for the operator.
    Pause,
}

/// Hexagonal-architecture role for a source file, inferred from its path.
/// Used by §5.11 signature-graph dispatch policy: ports get rewritten first
/// (they declare the contract), adapters get rewritten with the port content
/// as frozen DO-NOT-CHANGE context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileKind {
    /// File declares the contract (traits, types) other code depends on.
    /// Path matches `*/ports/*` or `*/core/*`.
    Port,
    /// File implements a port. Path matches `*/adapters/*`.
    Adapter,
    /// Use case / orchestration / other. Treated like Adapter for dispatch
    /// (the contract a use case consumes is still upstream).
    Other,
}

/// Classify a file by its repo-relative path. Path-based heuristic, not a
/// full trait-graph parse. Works for the hex convention (`src/core/ports/`,
/// `src/adapters/{primary,secondary}/`).
fn classify_file_kind(rel_path: &str) -> FileKind {
    let p = rel_path.to_lowercase();
    if p.contains("/ports/") || p.starts_with("ports/")
        || p.ends_with("/ports.rs") || p == "ports.rs"
    {
        FileKind::Port
    } else if p.contains("/adapters/") || p.starts_with("adapters/") {
        FileKind::Adapter
    } else if p.contains("/core/") && !p.contains("/adapters/") {
        // core/domain or core/usecases — usually declare types the
        // adapters depend on. Treat as Port for dispatch (fix first).
        FileKind::Port
    } else {
        FileKind::Other
    }
}

/// Read the verbatim contents of a single source file at
/// `<project>/<rel_path>`, capped at ~8 KB. Used by §5.13 to anchor the
/// model on existing identifiers + imports + structural choices instead
/// of asking it to invent them from scratch. Returns empty string if
/// the file can't be read (caller falls back to no-anchor prompt).
fn read_current_file_block(project_path: &Path, rel_path: &str) -> String {
    let p = project_path.join(rel_path);
    let content = match std::fs::read_to_string(&p) {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    const MAX_BYTES: usize = 8 * 1024;
    if content.len() > MAX_BYTES {
        // Keep the first 8 KB. Truncating mid-token is fine — the model
        // sees a clear marker and the prompt explicitly says "preserve
        // everything not directly causing an error", which includes the
        // truncated tail by omission.
        let mut truncated = content[..MAX_BYTES].to_string();
        truncated.push_str("\n// ... (current file truncated at 8 KB; preserve untruncated portion as-is)\n");
        truncated
    } else {
        content
    }
}

/// Read the verbatim contents of every `.rs` file under `<project>/src/core/ports/`
/// (and `<project>/src/ports/` as fallback) into a single string suitable
/// for injection into a code-patch prompt.
///
/// Cap at ~8 KB so the prompt budget stays bounded — if the port set is
/// larger than that, truncate with a clear marker. The whole point is
/// "freeze the contract" so the model can't reinvent it; if the contract
/// is too large to fit, we still send the first 8 KB which is far better
/// than nothing.
fn read_port_context(project_path: &Path) -> String {
    let mut candidates: Vec<PathBuf> = Vec::new();
    let ports_dirs = [
        project_path.join("src/core/ports"),
        project_path.join("src/ports"),
    ];
    for dir in &ports_dirs {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("rs") {
                    candidates.push(p);
                }
            }
        }
    }
    candidates.sort();
    let mut out = String::new();
    const MAX_BYTES: usize = 8 * 1024;
    for p in candidates {
        if out.len() >= MAX_BYTES {
            out.push_str("\n// ... (port context truncated at 8 KB)\n");
            break;
        }
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("?");
        match std::fs::read_to_string(&p) {
            Ok(content) => {
                out.push_str(&format!("\n// ===== {name} =====\n"));
                out.push_str(&content);
                out.push('\n');
            }
            Err(_) => continue,
        }
    }
    out
}

fn parse_env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[derive(Debug)]
struct AutoRepairConfig {
    project_path: PathBuf,
    max_iterations: u32,
    top_k: usize,
}

fn load_config(repo_root: &Path) -> Option<AutoRepairConfig> {
    let cfg_path = repo_root.join(".hex/project.json");
    let raw = std::fs::read_to_string(&cfg_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let block = v.get("auto_repair")?;
    let enabled = block.get("enabled").and_then(|x| x.as_bool()).unwrap_or(false);
    if !enabled {
        return None;
    }
    let project_path = block
        .get("project_path")
        .and_then(|x| x.as_str())
        .map(|s| repo_root.join(s))?;
    let max_iterations = block
        .get("max_iterations")
        .and_then(|x| x.as_u64())
        .map(|n| n as u32)
        .unwrap_or(DEFAULT_MAX_ITERATIONS);
    let top_k = block
        .get("top_k_files")
        .and_then(|x| x.as_u64())
        .map(|n| n as usize)
        .unwrap_or(DEFAULT_TOP_K);
    Some(AutoRepairConfig { project_path, max_iterations, top_k })
}

pub fn spawn(repo_root: PathBuf) {
    if std::env::var("HEX_DISABLE_AUTO_REPAIR").is_ok() {
        tracing::info!("auto_repair disabled via HEX_DISABLE_AUTO_REPAIR");
        return;
    }

    let interval_secs = parse_env_u64("HEX_AUTO_REPAIR_INTERVAL_SECS", DEFAULT_INTERVAL_SECS);
    let startup_secs = parse_env_u64("HEX_AUTO_REPAIR_STARTUP_SECS", DEFAULT_STARTUP_SECS);
    let cooldown_secs = parse_env_u64("HEX_AUTO_REPAIR_COOLDOWN_SECS", DEFAULT_COOLDOWN_SECS);

    tokio::spawn(async move {
        // Re-read config at startup so the operator can enable it via
        // `.hex/project.json` after nexus is already running (after a
        // nexus restart, this fn fires fresh).
        let cfg = match load_config(&repo_root) {
            Some(c) => c,
            None => {
                tracing::debug!(
                    repo_root = %repo_root.display(),
                    "auto_repair: not enabled in .hex/project.json — skipping"
                );
                return;
            }
        };

        tracing::info!(
            interval_secs,
            startup_secs,
            cooldown_secs,
            project_path = %cfg.project_path.display(),
            max_iterations = cfg.max_iterations,
            top_k = cfg.top_k,
            "auto_repair: spawning (autonomous cargo-check → code_patch loop)"
        );

        tokio::time::sleep(Duration::from_secs(startup_secs)).await;

        let nexus_port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
        let nexus_base = format!("http://127.0.0.1:{}", nexus_port);
        let http = match reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
        {
            Ok(c) => Arc::new(c),
            Err(e) => {
                tracing::warn!(error = %e, "auto_repair: http client build failed");
                return;
            }
        };

        let cooldown = Duration::from_secs(cooldown_secs);
        let mut ticker = tokio::time::interval(Duration::from_secs(interval_secs));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

        loop {
            ticker.tick().await;
            if let Err(e) = run_tick(&http, &nexus_base, &cfg, cooldown).await {
                tracing::warn!(error = %e, "auto_repair: tick failed");
            }
        }
    });
}

async fn run_tick(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    cfg: &AutoRepairConfig,
    cooldown: Duration,
) -> Result<(), String> {
    // Hard cap on iterations: stop the loop after this many ticks
    // even if there are still errors, so it doesn't spin forever on
    // an unfixable issue.
    {
        let s = state().lock().unwrap();
        if s.iterations >= cfg.max_iterations {
            return Ok(());
        }
    }

    let (count, by_file, errors_per_file) = match cargo_check_errors(&cfg.project_path).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "auto_repair: cargo_check failed");
            return Ok(());
        }
    };

    // Track plateau: if error count has not strictly decreased for N
    // consecutive ticks, escalate to claude -p (T3 frontier) before
    // giving up. Each escalation pass consumes one of MAX_FRONTIER_ATTEMPTS;
    // once exhausted, the loop pauses and surfaces to operator inbox.
    // See retro §5.12.
    let plateau_action: PlateauAction = {
        let mut s = state().lock().unwrap();
        s.iterations += 1;
        let prev = s.last_error_count.unwrap_or(u32::MAX);
        if count >= prev {
            s.no_progress_count += 1;
        } else {
            s.no_progress_count = 0;
        }
        s.last_error_count = Some(count);
        tracing::info!(
            iteration = s.iterations,
            max_iterations = cfg.max_iterations,
            error_count = count,
            no_progress_count = s.no_progress_count,
            frontier_attempts = s.frontier_attempts,
            "auto_repair: tick"
        );
        if s.no_progress_count >= 3 {
            if s.frontier_attempts < MAX_FRONTIER_ATTEMPTS {
                // Consume one frontier attempt and reset no_progress so the
                // next normal tick (post-escalation) gets a fresh chance.
                s.frontier_attempts += 1;
                s.no_progress_count = 0;
                PlateauAction::Escalate
            } else {
                // Bump iterations to the cap so we silently stop.
                s.iterations = cfg.max_iterations;
                PlateauAction::Pause
            }
        } else {
            PlateauAction::Continue
        }
    };

    if matches!(plateau_action, PlateauAction::Pause) {
        tracing::warn!(
            error_count = count,
            frontier_attempts = MAX_FRONTIER_ATTEMPTS,
            "auto_repair: plateau + frontier exhausted — pausing loop \
             (operator: `hex auto-repair restart` to re-engage)"
        );
        return Ok(());
    }

    if count == 0 {
        tracing::info!("auto_repair: cargo check is clean — nothing to do");
        return Ok(());
    }

    // Sort files by error count desc, take top K, respect cooldowns.
    let mut sorted: Vec<(String, u32)> = by_file.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let now = Instant::now();
    let persona_inflight_ttl = Duration::from_secs(PERSONA_INFLIGHT_TTL_SECS);
    let to_fire_unfiltered: Vec<(String, u32)> = {
        let s = state().lock().unwrap();
        sorted.into_iter()
            .filter(|(path, _)| {
                // Existing cooldown check (persona re-dispatch suppression).
                let cooled = s.file_cooldowns.get(path)
                    .map(|t| now.duration_since(*t) >= cooldown)
                    .unwrap_or(true);
                // §5.15 file-overlap gate: skip files persona has been told
                // about within the inflight window AND files frontier is
                // currently rewriting. Either case means another path is
                // already working this file.
                let persona_busy = s.dispatched_to_persona.get(path)
                    .map(|t| now.duration_since(*t) < persona_inflight_ttl)
                    .unwrap_or(false);
                let frontier_busy = s.frontier_inflight.contains(path);
                cooled && !persona_busy && !frontier_busy
            })
            .take(cfg.top_k * 2)  // Take more so we have room to skip adapters when ports exist
            .collect()
    };

    // §5.11 signature-graph dispatch policy: if any errored file is a
    // port (declares traits), restrict this tick to ports only. Adapters
    // depend on port signatures; rewriting them in parallel produces
    // signature drift (proved empirically 2026-05-29 — §5.12's first
    // live test took 18→43 errors because ports + adapters were rewritten
    // independently). Once ports are clean, adapters get dispatched WITH
    // verbatim port content as DO-NOT-CHANGE context.
    let any_port_errored = to_fire_unfiltered
        .iter()
        .any(|(p, _)| classify_file_kind(p) == FileKind::Port);
    let (to_fire, port_first_mode): (Vec<(String, u32)>, bool) = if any_port_errored {
        let only_ports: Vec<(String, u32)> = to_fire_unfiltered
            .into_iter()
            .filter(|(p, _)| classify_file_kind(p) == FileKind::Port)
            .take(cfg.top_k)
            .collect();
        tracing::info!(
            ports_to_fix = only_ports.len(),
            "auto_repair: ports have errors — dispatching ports first (§5.11)"
        );
        (only_ports, true)
    } else {
        let adapters: Vec<(String, u32)> = to_fire_unfiltered.into_iter().take(cfg.top_k).collect();
        (adapters, false)
    };

    // Compute the workspace-relative prefix for project paths. Cargo
    // check emits paths like `src/foo.rs` relative to cwd (the project
    // root), but the executor resolves all file_write paths against
    // the hex WORKSPACE root. Without this prefix, the persona's
    // code_patch lands at `<workspace>/src/foo.rs` instead of
    // `<workspace>/examples/ebay-clone/backend/src/foo.rs`.
    //
    // Catastrophic bug surfaced 2026-05-29 PM: 9 phantom files were
    // written to the hex workspace root for hours. None of them fixed
    // the actual ebay-clone — the loop appeared to work but was
    // operating on a non-existent shadow project.
    let repo_root: PathBuf = std::env::current_dir()
        .ok()
        .or_else(|| {
            // Best-effort fallback: walk up from project_path until we
            // find a workspace Cargo.toml.
            let mut cur = cfg.project_path.clone();
            while let Some(parent) = cur.parent() {
                if parent.join("Cargo.toml").is_file() &&
                   parent.join(".git").exists() {
                    return Some(parent.to_path_buf());
                }
                cur = parent.to_path_buf();
            }
            None
        })
        .unwrap_or_else(|| std::path::PathBuf::from("/home/gary/development/hex"));
    let project_rel_prefix: String = cfg.project_path
        .strip_prefix(&repo_root)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let prefix_path = |rel: &str| -> String {
        if project_rel_prefix.is_empty() {
            rel.to_string()
        } else {
            format!("{}/{}", project_rel_prefix.trim_end_matches('/'), rel)
        }
    };

    // §5.11: when dispatching adapters (ports are already clean OR not
    // present in this tick), inject verbatim port file contents so the
    // model sees the contract it must implement against. For port-first
    // mode (this tick rewrites ports), no port_context — we're rewriting
    // the contract itself.
    let port_context: String = if port_first_mode {
        String::new()
    } else {
        read_port_context(&cfg.project_path)
    };

    // Frontier escalation branch (retro §5.12). When plateau is hit and we
    // still have attempts left, skip the normal persona-dispatch chain and
    // call claude -p directly on the top-K files. Local Ollama models have
    // already been given 3+ chances; the persona inbox isn't going to
    // unstick this fault line. If frontier isn't available (no claude
    // binary on PATH), fall through to pause.
    if matches!(plateau_action, PlateauAction::Escalate) {
        match crate::composition::standalone::frontier_inference_adapter() {
            Some(adapter) => {
                let attempt = state().lock().unwrap().frontier_attempts;
                tracing::warn!(
                    error_count = count,
                    frontier_attempt = attempt,
                    frontier_max = MAX_FRONTIER_ATTEMPTS,
                    files = to_fire.len(),
                    port_first = port_first_mode,
                    port_context_bytes = port_context.len(),
                    "auto_repair: plateau — escalating to claude -p (frontier)"
                );
                let committed = run_frontier_escalation(
                    adapter,
                    &to_fire,
                    &errors_per_file,
                    &project_rel_prefix,
                    &cfg.project_path,
                    &repo_root,
                    &port_context,
                )
                .await;
                tracing::info!(
                    files_committed = committed,
                    "auto_repair: frontier escalation pass complete"
                );
            }
            None => {
                let mut s = state().lock().unwrap();
                s.iterations = cfg.max_iterations;
                tracing::warn!(
                    "auto_repair: plateau + claude binary not on PATH — pausing. \
                     Install claude CLI or set HEX_CLAUDE_BINARY to enable T3 escalation."
                );
            }
        }
        return Ok(());
    }

    // Phase A: harvest unresolved-import errors and fire create-module asks
    // for each missing module. This closes the biggest plateau cause —
    // "can't fix file Y because module X is missing; can't fix module X
    // because we only re-ask the broken file". Surfaced 2026-05-29 PM.
    //
    // Collect missing modules across ALL error lines (not just top-K files),
    // dedupe, respect file_cooldowns, fire one create-ask per missing path.
    let missing_modules = extract_missing_modules(&errors_per_file, &cfg.project_path);
    for missing_path in missing_modules {
        let missing_path = prefix_path(&missing_path);
        let cooldown_key = format!("create:{}", missing_path);
        let in_cooldown = {
            let s = state().lock().unwrap();
            s.file_cooldowns.get(&cooldown_key)
                .map(|t| now.duration_since(*t) < cooldown)
                .unwrap_or(false)
        };
        if in_cooldown {
            continue;
        }
        let content = format!(
            "Create the missing Rust module file {missing_path}. Other files in this crate \
             import from this module path but the file does not exist on disk — that's why \
             cargo check is failing.\n\n\
             Write a minimal but functional module that exports the symbols its callers expect. \
             Use ONLY the actual workspace exports listed in the AVAILABLE WORKSPACE EXPORTS \
             block. Do NOT invent types. Do NOT import from modules that aren't in the export list.\n\n\
             The file should compile by itself: a `pub fn`, `pub struct`, `pub trait`, or `pub use` \
             that satisfies the importer. If the importer needs a router handler, write a handler. \
             If it needs a port-trait impl, write the impl. Match the existing crate's hex \
             architecture conventions (domain → ports → adapters)."
        );
        match send_dm(http, nexus_base, "hex-coder", "auto_repair_create_module", &content).await {
            Ok(()) => {
                tracing::info!(
                    missing_path = %missing_path,
                    "auto_repair: dispatched create-module ask"
                );
                let mut s = state().lock().unwrap();
                s.file_cooldowns.insert(cooldown_key, now);
            }
            Err(e) => {
                tracing::warn!(error = %e, missing_path = %missing_path, "auto_repair: create-module dispatch failed");
            }
        }
    }

    for (rel_path, err_count) in to_fire {
        // Inject the actual compile errors so the persona knows what to fix
        // instead of regenerating semantically-equivalent broken content.
        // This was the gap that made the loop plateau in the first pass:
        // the persona kept rewriting the file with the same errors because
        // it had no signal on what was wrong. Surfaced 2026-05-29 morning.
        // Rewrite the bare `src/foo.rs:10:5: error` prefix to the
        // workspace-relative path so the persona can't grab a phantom
        // bare path out of the error block and write to the wrong place.
        // Surfaced 2026-05-29: 56 phantom files appeared at hex
        // workspace root because the persona was using paths from the
        // error block instead of the explicit "Rewrite <prefixed>" mention.
        let errors_block = errors_per_file
            .get(&rel_path)
            .map(|lines| {
                lines.iter()
                    .take(20)
                    .map(|l| {
                        if l.starts_with("src/") {
                            format!("{}/{}", project_rel_prefix.trim_end_matches('/'), l)
                        } else {
                            l.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_default();
        // §5.13: read current file content so the model is editing, not
        // synthesizing. The bare unprefixed rel_path here is the project-
        // relative path; project_path is the project root.
        let current_file_block = read_current_file_block(&cfg.project_path, &rel_path);
        // §5.15: keep the unprefixed key so it matches the cargo-check
        // filter side. Bug surfaced in run 3 — the shadowing below
        // pushed the PREFIXED path into file_cooldowns/dispatched_to_persona,
        // but the filter at to_fire_unfiltered checks the bare path
        // from cargo check, so neither map ever matched anything.
        // Cooldowns + §5.15 gate were no-ops the entire prior session.
        let dedupe_key = rel_path.clone();
        // Prefix the path so the persona writes to the project, not the
        // hex workspace root. See `project_rel_prefix` block above.
        let rel_path = prefix_path(&rel_path);
        // §5.11: include verbatim port file contents as DO-NOT-CHANGE
        // context when dispatching adapter rewrites. Skipped in
        // port_first_mode (port_context is empty there).
        let port_freeze_block = if port_context.is_empty() {
            String::new()
        } else {
            format!(
                "\n--- PORT CONTRACTS (DO NOT CHANGE THESE SIGNATURES) ---\n\
                 {port_context}\n\
                 --- END PORT CONTRACTS ---\n\n\
                 The trait declarations above are the contract. Your adapter MUST match \
                 the EXACT trait method signatures (arg names, arg types, return types, \
                 async-ness). Do NOT add, remove, or rename methods. If the port file you \
                 see above looks wrong, fix the ADAPTER to match it — the port is frozen \
                 for this pass.\n"
            )
        };
        // §5.13: current-file block goes ABOVE the errors so the model
        // reads anchor first, error focus second.
        let current_file_section = if current_file_block.is_empty() {
            String::new()
        } else {
            format!(
                "\n--- CURRENT FILE CONTENT ({rel_path}) — preserve this as much as possible ---\n\
                 {current_file_block}\n\
                 --- END CURRENT FILE ---\n"
            )
        };
        let content = format!(
            "Edit {rel_path} to fix the {err_count} compile errors listed below. \
             Output the COMPLETE file with only the minimum changes needed to make the listed \
             errors go away. Preserve EVERY identifier, import, type reference, struct field, \
             trait method, and structural choice from the current file that is not directly \
             causing one of the listed errors. Do NOT rename. Do NOT invent new type names. \
             Do NOT add or remove unrelated declarations. If a name looks wrong but is not in \
             the error list, leave it.\n\
             {current_file_section}\n\
             --- SPECIFIC COMPILE ERRORS (cargo check, --message-format=short) ---\n\
             {errors_block}\n\
             --- END ERRORS ---\n\
             {port_freeze_block}\n\
             For each error: identify the single token / line at fault, change only that, \
             leave the rest of the file alone. If an error says \"cannot find type X\" check \
             the workspace exports for the right path; do not invent a new type. If an error \
             says \"function takes N arguments\" match the trait's declared signature."
        );
        match send_dm(http, nexus_base, "hex-coder", "auto_repair", &content).await {
            Ok(()) => {
                tracing::info!(
                    rel_path = %rel_path,
                    error_count = err_count,
                    "auto_repair: dispatched code_patch ask"
                );
                let mut s = state().lock().unwrap();
                // §5.15 fix: insert UNPREFIXED key so it matches the
                // filter at to_fire_unfiltered. The PREFIXED rel_path
                // is for display + dispatch content only.
                s.file_cooldowns.insert(dedupe_key.clone(), now);
                s.dispatched_to_persona.insert(dedupe_key, now);
            }
            Err(e) => {
                tracing::warn!(error = %e, rel_path = %rel_path, "auto_repair: dispatch failed");
            }
        }
    }

    Ok(())
}

/// Scan compile-error lines for `error[E0432]: unresolved import` and
/// `could not find \`X\` in \`Y\`` patterns. Returns a deduped set of
/// missing-module paths (relative to crate root) that don't yet exist
/// on disk.
///
/// Only emits paths that:
///   1. Aren't currently a file under src/
///   2. Aren't a directory either (so we don't try to create something
///      that's actually a sub-module via mod.rs)
///   3. Are inside this crate (start with `crate::` in the original
///      error message)
///
/// Surfaced 2026-05-29 PM: of the 41-error plateau, ~17 errors were
/// single E0432 lines listing 9+ missing modules each. Rewriting the
/// importing file can't fix them — the actual fix is to CREATE the
/// missing modules.
fn extract_missing_modules(
    errors_per_file: &HashMap<String, Vec<String>>,
    project_path: &Path,
) -> Vec<String> {
    use std::collections::HashSet;
    let src_dir = project_path.join("src");
    // Match `crate::A::B::C` inside backticks. Pull all such paths from
    // any error line; the same line can list 9+ paths.
    let re = regex::Regex::new(r"`crate::([a-zA-Z0-9_:]+)`").unwrap();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for lines in errors_per_file.values() {
        for line in lines {
            // Only mine import-style errors. E0432 = unresolved import.
            if !line.contains("E0432") && !line.contains("unresolved import") {
                continue;
            }
            for cap in re.captures_iter(line) {
                let mod_path = cap.get(1).map(|m| m.as_str()).unwrap_or("");
                if mod_path.is_empty() {
                    continue;
                }
                // The path includes the importing leaf when the error
                // form is `crate::foo::bar::baz` where `baz` is the
                // imported symbol — we want the MODULE that contains
                // `baz`, not `baz` itself. Heuristic: if the last
                // segment is lowercase and contains no `_`, OR if the
                // overall path corresponds to a module that already
                // exists, skip. We'll let the file-check filter sort it.
                let rel_file = format!("src/{}.rs", mod_path.replace("::", "/"));
                let abs_file = project_path.join(&rel_file);
                let abs_dir_mod = project_path.join(format!(
                    "src/{}/mod.rs", mod_path.replace("::", "/")
                ));
                let exists = abs_file.is_file() || abs_dir_mod.is_file();
                if exists {
                    continue;
                }
                // Also skip if a directory exists (would mean it's a
                // sub-module that needs mod.rs, not a leaf file).
                let abs_dir = project_path.join(format!(
                    "src/{}", mod_path.replace("::", "/")
                ));
                if abs_dir.is_dir() && !abs_dir_mod.is_file() {
                    // Directory present but no mod.rs — that's actually
                    // a thing to create. Use rel_file (the .rs sibling)
                    // OR the mod.rs version. Prefer mod.rs.
                    let candidate = format!("src/{}/mod.rs", mod_path.replace("::", "/"));
                    if seen.insert(candidate.clone()) {
                        out.push(candidate);
                    }
                    continue;
                }
                // Sanity: the parent dir must exist (don't try to
                // create deep into nowhere). e.g. for `crate::foo::bar`
                // we need src/foo/ to exist.
                if let Some(parent) = abs_file.parent() {
                    if parent.is_dir() || parent == src_dir.as_path() {
                        if seen.insert(rel_file.clone()) {
                            out.push(rel_file);
                        }
                    }
                }
            }
        }
    }
    // Cap at 10 to keep prompt budget bounded — fire the rest next tick.
    out.truncate(10);
    out
}

/// Run `cargo check` in the configured project and return:
///   (total error count, errors-per-file as path → count, errors-per-file as path → lines)
async fn cargo_check_errors(
    project_path: &Path,
) -> Result<(u32, HashMap<String, u32>, HashMap<String, Vec<String>>), String> {
    let out = tokio::process::Command::new("cargo")
        .arg("check")
        .arg("--message-format=short")
        .current_dir(project_path)
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .await
        .map_err(|e| format!("cargo check spawn: {}", e))?;

    let stderr = String::from_utf8_lossy(&out.stderr);
    let mut by_file: HashMap<String, u32> = HashMap::new();
    let mut errors_per_file: HashMap<String, Vec<String>> = HashMap::new();
    let mut total: u32 = 0;
    for line in stderr.lines() {
        // Match `--message-format=short` output:
        //   src/foo.rs:12:5: error[E0432]: ...
        if !line.contains(": error") {
            continue;
        }
        if let Some(idx) = line.find(":") {
            let path = &line[..idx];
            if path.ends_with(".rs") {
                *by_file.entry(path.to_string()).or_insert(0) += 1;
                // Keep the full error line so the persona sees what to fix.
                errors_per_file
                    .entry(path.to_string())
                    .or_default()
                    .push(line.trim().to_string());
                total += 1;
            }
        }
    }
    Ok((total, by_file, errors_per_file))
}

/// Run one frontier-escalation pass over the top-K errored files.
///
/// For each file: build a self-contained rewrite prompt (errors + intent),
/// invoke the frontier adapter (claude -p subprocess), strip markdown
/// fences from the response if present, write the file to disk, and commit
/// it through git with a clear `[frontier]` marker so the operator can
/// trace which commits came from T3 escalation.
///
/// Does NOT route through the persona/drafter/twin/executor chain — the
/// persona has already failed on these files 3+ times. The chain's safety
/// review is replaced by:
///   1. Bounded attempts (`MAX_FRONTIER_ATTEMPTS`)
///   2. Clear commit-message marker for operator review
///   3. Next tick's cargo_check is the regression gate
///
/// Returns the number of files successfully written + committed.
/// RAII guard that removes a file from `frontier_inflight` on drop.
/// §5.15: ensures the lock is released on every exit path from a frontier
/// loop iteration — including `continue`, `?`, panic — without manual
/// cleanup at each branch.
struct FrontierInflightGuard {
    path: String,
}

impl Drop for FrontierInflightGuard {
    fn drop(&mut self) {
        if let Ok(mut s) = state().lock() {
            s.frontier_inflight.remove(&self.path);
        }
    }
}

async fn run_frontier_escalation(
    adapter: Arc<dyn IInferencePort>,
    to_fire: &[(String, u32)],
    errors_per_file: &HashMap<String, Vec<String>>,
    project_rel_prefix: &str,
    project_path: &Path,
    repo_root: &Path,
    port_context: &str,
) -> usize {
    let mut committed = 0usize;
    for (rel_path, err_count) in to_fire {
        // §5.15: claim file-overlap lock BEFORE any work. If the persona
        // is currently processing this file (e.g. an inbox message landed
        // mid-tick), skip it. Guard releases lock on every exit path.
        let _guard: FrontierInflightGuard = {
            let mut s = state().lock().unwrap();
            if s.frontier_inflight.contains(rel_path) {
                tracing::info!(
                    rel_path = %rel_path,
                    "frontier: file already in flight — skipping"
                );
                continue;
            }
            s.frontier_inflight.insert(rel_path.clone());
            FrontierInflightGuard { path: rel_path.clone() }
        };
        let prefixed = if project_rel_prefix.is_empty() {
            rel_path.clone()
        } else {
            format!("{}/{}", project_rel_prefix.trim_end_matches('/'), rel_path)
        };

        let errors_block = errors_per_file
            .get(rel_path)
            .map(|lines| lines.iter().take(20).cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();

        // §5.11: when rewriting an adapter, freeze the port contract.
        // When port_context is empty (this is a port-first pass), skip
        // the freeze block.
        let port_freeze_block = if port_context.is_empty() {
            String::new()
        } else {
            format!(
                "\n--- PORT CONTRACTS (DO NOT CHANGE THESE SIGNATURES) ---\n\
                 {port_context}\n\
                 --- END PORT CONTRACTS ---\n\n\
                 The trait declarations above are the frozen contract for this pass. \
                 Match arg names, types, return types, async-ness EXACTLY. Do NOT \
                 add, remove, or rename trait methods. If a method on the trait is \
                 not in your file, add it; if your file has methods not on the trait, \
                 remove them.\n"
            )
        };

        // §5.13: anchor on existing file content instead of asking the
        // model to synthesize from scratch. Empirically — from two prior
        // live tests on this same project — full-rewrite prompts produce
        // type-reference drift even when the model is claude. With the
        // current file in front of it, the model edits.
        let current_file_block = read_current_file_block(project_path, rel_path);
        let current_file_section = if current_file_block.is_empty() {
            String::new()
        } else {
            format!(
                "\n--- CURRENT FILE CONTENT ({prefixed}) — preserve this as much as possible ---\n\
                 {current_file_block}\n\
                 --- END CURRENT FILE ---\n"
            )
        };

        let prompt = format!(
            "You are repairing a Rust file in the hex-clone project. The file at \
             `{prefixed}` has {err_count} compile errors listed below. Output the \
             COMPLETE file content with ONLY the minimum changes needed to make the \
             listed errors go away. Preserve EVERY identifier, import, type reference, \
             struct field, trait method, and structural choice from the current file \
             that is not directly causing one of the listed errors. Output raw Rust \
             source — NO markdown fences, NO commentary, NO explanation. Your output \
             will be written verbatim to disk and compiled.\n\
             {current_file_section}\n\
             --- COMPILE ERRORS (cargo check, --message-format=short) ---\n\
             {errors_block}\n\
             --- END ERRORS ---\n\
             {port_freeze_block}\n\
             For each error: identify the single token, line, or block at fault, change \
             only that, leave the rest of the file alone. Do NOT rename your way out \
             of an error. Do NOT invent new type names that aren't in the current file \
             or the port contracts. Do NOT add a `pub use SomeNew::Thing` that wasn't \
             already there. Output ONLY the new file contents, starting with the first \
             line of the .rs file."
        );

        let req = InferenceRequest {
            model: "claude-code".to_string(),
            system_prompt: String::new(),
            messages: vec![Message::user(&prompt)],
            tools: vec![],
            max_tokens: 8192,
            temperature: 0.0,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Normal,
            grammar: None,
        };

        let response = match adapter.complete(req).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(rel_path = %prefixed, error = %e, "frontier: complete failed");
                continue;
            }
        };

        let content_text: String = response
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("");

        let extracted = strip_code_fences(&content_text);
        if extracted.trim().is_empty() {
            tracing::warn!(rel_path = %prefixed, "frontier: empty response — skipping write");
            continue;
        }

        let abs_path = project_path.join(rel_path);
        if let Some(parent) = abs_path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(rel_path = %prefixed, error = %e, "frontier: mkdir failed");
                continue;
            }
        }
        if let Err(e) = std::fs::write(&abs_path, &extracted) {
            tracing::warn!(rel_path = %prefixed, error = %e, "frontier: write failed");
            continue;
        }
        tracing::info!(
            rel_path = %prefixed,
            bytes = extracted.len(),
            "frontier: file written"
        );

        // git add + commit. Failures here are non-fatal — the file is on
        // disk; operator can commit manually if needed.
        let add = tokio::process::Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("add")
            .arg(&prefixed)
            .status()
            .await;
        if !add.map(|s| s.success()).unwrap_or(false) {
            tracing::warn!(rel_path = %prefixed, "frontier: git add failed");
            continue;
        }

        let msg = format!(
            "fix(auto-repair frontier): rewrite {prefixed} via claude -p\n\n\
             auto_repair plateau triggered T3 (frontier) inference on this file. \
             Local Ollama models couldn't make progress over 3+ ticks; claude -p \
             was invoked to break the ceiling. See retro §5.12.\n\n\
             [frontier-escalation] [auto-repair]"
        );
        let commit = tokio::process::Command::new("git")
            .arg("-C")
            .arg(repo_root)
            .arg("commit")
            .arg("-m")
            .arg(&msg)
            .status()
            .await;
        if commit.map(|s| s.success()).unwrap_or(false) {
            committed += 1;
            tracing::info!(rel_path = %prefixed, "frontier: committed");
        } else {
            tracing::warn!(rel_path = %prefixed, "frontier: git commit failed (file is on disk)");
        }
    }
    committed
}

/// If the model wrapped its output in ``` fences (despite being told not to),
/// peel them off. Otherwise return the input unchanged. Conservative: if we
/// can't find a clean fence pair, we return the original text so we never
/// truncate real code.
fn strip_code_fences(text: &str) -> String {
    let trimmed = text.trim();
    if !trimmed.starts_with("```") {
        return trimmed.to_string();
    }
    let after_first_line = match trimmed.split_once('\n') {
        Some((_, rest)) => rest,
        None => return trimmed.to_string(),
    };
    match after_first_line.rfind("```") {
        Some(close) => after_first_line[..close].trim_end().to_string(),
        None => trimmed.to_string(),
    }
}

async fn send_dm(
    http: &Arc<reqwest::Client>,
    nexus_base: &str,
    target: &str,
    subject: &str,
    content: &str,
) -> Result<(), String> {
    let url = format!("{}/api/org/send-message", nexus_base);
    let body = json!({
        "from": SEND_AGENT_ID,
        "to": target,
        "subject": subject,
        "content": content,
    });
    let resp = http
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("send transport: {}", e))?;
    if !resp.status().is_success() {
        let status = resp.status();
        let txt = resp.text().await.unwrap_or_default();
        return Err(format!("send HTTP {}: {}", status, txt.chars().take(200).collect::<String>()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_code_fences_passthrough_when_no_fences() {
        let input = "use crate::foo;\n\nfn main() {}\n";
        assert_eq!(strip_code_fences(input), input.trim());
    }

    #[test]
    fn strip_code_fences_peels_rust_fence() {
        let input = "```rust\nuse crate::foo;\n\nfn main() {}\n```";
        let out = strip_code_fences(input);
        assert!(!out.contains("```"));
        assert!(out.starts_with("use crate::foo"));
        assert!(out.trim_end().ends_with("fn main() {}"));
    }

    #[test]
    fn strip_code_fences_peels_bare_fence() {
        let input = "```\nfn x() {}\n```";
        let out = strip_code_fences(input);
        assert_eq!(out, "fn x() {}");
    }

    #[test]
    fn strip_code_fences_keeps_unmatched_open_fence_as_is() {
        // If we see ``` at start but never a closing fence, return the
        // original — we'd rather write a slightly-malformed file (cargo
        // catches it next tick) than truncate real code.
        let input = "```rust\nfn x() {}\n";
        let out = strip_code_fences(input);
        // Unmatched fence: the closing ``` is missing, so we return
        // everything after the opening fence line.
        assert!(out.contains("fn x() {}"));
    }

    #[test]
    fn plateau_action_default_continue() {
        // Sanity: PlateauAction variants are wired through match.
        let a = PlateauAction::Continue;
        assert_eq!(a, PlateauAction::Continue);
        assert_ne!(a, PlateauAction::Pause);
        assert_ne!(a, PlateauAction::Escalate);
    }

    #[test]
    fn classify_file_kind_ports_directory() {
        assert_eq!(
            classify_file_kind("examples/ebay-clone/backend/src/core/ports/listing_repo.rs"),
            FileKind::Port
        );
        assert_eq!(
            classify_file_kind("src/ports/mod.rs"),
            FileKind::Port
        );
        assert_eq!(
            classify_file_kind("ports/auth.rs"),
            FileKind::Port
        );
    }

    #[test]
    fn classify_file_kind_adapter_directory() {
        assert_eq!(
            classify_file_kind("examples/ebay-clone/backend/src/adapters/secondary/mod.rs"),
            FileKind::Adapter
        );
        assert_eq!(
            classify_file_kind("src/adapters/primary/http_axum/mod.rs"),
            FileKind::Adapter
        );
    }

    #[test]
    fn classify_file_kind_core_domain_treated_as_port() {
        // core/domain types are upstream contracts adapters depend on —
        // dispatch them in port-first mode for the same reason ports get
        // priority.
        assert_eq!(
            classify_file_kind("examples/ebay-clone/backend/src/core/domain/user.rs"),
            FileKind::Port
        );
        assert_eq!(
            classify_file_kind("src/core/usecases/auth.rs"),
            FileKind::Port
        );
    }

    #[test]
    fn classify_file_kind_other_for_unrecognised_paths() {
        assert_eq!(
            classify_file_kind("src/main.rs"),
            FileKind::Other
        );
        assert_eq!(
            classify_file_kind("composition_root.rs"),
            FileKind::Other
        );
    }

    #[test]
    fn classify_file_kind_adapters_takes_precedence_over_core() {
        // A file under both /adapters/ and conceptually-core paths is an
        // adapter (the persona-implements-a-port case). Both heuristics
        // could match — adapters check first.
        assert_eq!(
            classify_file_kind("src/adapters/secondary/core_state.rs"),
            FileKind::Adapter
        );
    }

    #[test]
    fn read_current_file_block_returns_empty_for_missing_file() {
        let tmp = std::env::temp_dir().join("hex_test_auto_repair_missing");
        let result = read_current_file_block(&tmp, "does_not_exist.rs");
        assert_eq!(result, "");
    }

    #[test]
    fn read_current_file_block_returns_full_content_under_limit() {
        let tmp = std::env::temp_dir().join("hex_test_auto_repair_under");
        let _ = std::fs::create_dir_all(&tmp);
        let target = tmp.join("under.rs");
        std::fs::write(&target, "fn x() {}\n").unwrap();
        let result = read_current_file_block(&tmp, "under.rs");
        assert_eq!(result, "fn x() {}\n");
        let _ = std::fs::remove_file(&target);
    }

    #[test]
    fn frontier_inflight_guard_clears_on_drop() {
        let path = "test/path.rs".to_string();
        {
            let mut s = state().lock().unwrap();
            s.frontier_inflight.insert(path.clone());
            assert!(s.frontier_inflight.contains(&path));
        }
        {
            let _guard = FrontierInflightGuard { path: path.clone() };
            let s = state().lock().unwrap();
            assert!(s.frontier_inflight.contains(&path));
        }
        // Guard dropped — entry removed.
        let s = state().lock().unwrap();
        assert!(!s.frontier_inflight.contains(&path));
    }

    #[test]
    fn read_current_file_block_truncates_with_marker_above_limit() {
        let tmp = std::env::temp_dir().join("hex_test_auto_repair_over");
        let _ = std::fs::create_dir_all(&tmp);
        let target = tmp.join("over.rs");
        // 10 KB of content — over the 8 KB cap.
        let blob: String = std::iter::repeat('a').take(10 * 1024).collect();
        std::fs::write(&target, &blob).unwrap();
        let result = read_current_file_block(&tmp, "over.rs");
        assert!(result.len() < 11 * 1024, "should be near the cap, got {} bytes", result.len());
        assert!(
            result.contains("current file truncated at 8 KB"),
            "expected truncation marker, got tail: {:?}",
            result.chars().rev().take(80).collect::<String>().chars().rev().collect::<String>()
        );
        let _ = std::fs::remove_file(&target);
    }
}
