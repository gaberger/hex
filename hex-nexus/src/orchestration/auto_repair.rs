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

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;

const DEFAULT_INTERVAL_SECS: u64 = 120;
const DEFAULT_STARTUP_SECS: u64 = 90;
const DEFAULT_COOLDOWN_SECS: u64 = 240;
const DEFAULT_MAX_ITERATIONS: u32 = 20;
const DEFAULT_TOP_K: usize = 3;
const SEND_AGENT_ID: &str = "nexus-auto-repair";

#[derive(Default)]
struct RepairState {
    iterations: u32,
    last_error_count: Option<u32>,
    no_progress_count: u32,
    file_cooldowns: HashMap<String, Instant>,
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
}

pub fn snapshot() -> RepairStateSnapshot {
    let s = state().lock().unwrap();
    let max_it = std::env::var("HEX_AUTO_REPAIR_MAX_ITERATIONS_OVERRIDE")
        .ok()
        .and_then(|v| v.parse().ok());
    let paused = s.no_progress_count >= 3
        || s.iterations >= max_it.unwrap_or(u32::MAX);
    RepairStateSnapshot {
        iterations: s.iterations,
        last_error_count: s.last_error_count,
        no_progress_count: s.no_progress_count,
        paused,
        max_iterations: max_it,
        file_cooldowns_count: s.file_cooldowns.len(),
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
    tracing::info!("auto_repair: state reset via operator API");
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
    // consecutive ticks, stop firing asks (we're not helping). The
    // operator can manually re-enable with a nexus restart.
    {
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
            "auto_repair: tick"
        );
        if s.no_progress_count >= 3 {
            tracing::warn!(
                error_count = count,
                "auto_repair: 3 consecutive ticks without progress — pausing loop (restart nexus to re-engage)"
            );
            // Bump iterations to the cap so we silently stop.
            s.iterations = cfg.max_iterations;
            return Ok(());
        }
    }

    if count == 0 {
        tracing::info!("auto_repair: cargo check is clean — nothing to do");
        return Ok(());
    }

    // Sort files by error count desc, take top K, respect cooldowns.
    let mut sorted: Vec<(String, u32)> = by_file.into_iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(&a.1));

    let now = Instant::now();
    let to_fire: Vec<(String, u32)> = {
        let s = state().lock().unwrap();
        sorted.into_iter()
            .filter(|(path, _)| {
                s.file_cooldowns.get(path)
                    .map(|t| now.duration_since(*t) >= cooldown)
                    .unwrap_or(true)
            })
            .take(cfg.top_k)
            .collect()
    };

    // Phase A: harvest unresolved-import errors and fire create-module asks
    // for each missing module. This closes the biggest plateau cause —
    // "can't fix file Y because module X is missing; can't fix module X
    // because we only re-ask the broken file". Surfaced 2026-05-29 PM.
    //
    // Collect missing modules across ALL error lines (not just top-K files),
    // dedupe, respect file_cooldowns, fire one create-ask per missing path.
    let missing_modules = extract_missing_modules(&errors_per_file, &cfg.project_path);
    for missing_path in missing_modules {
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
        let errors_block = errors_per_file
            .get(&rel_path)
            .map(|lines| lines.iter().take(20).cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default();
        let content = format!(
            "Rewrite {rel_path} to fix the {err_count} compile errors listed below. The current \
             file is BROKEN. Use only the actual workspace exports listed in the AVAILABLE \
             WORKSPACE EXPORTS block. Preserve the file's intent (which types / traits / \
             handlers it should expose) but regenerate the body so the SPECIFIC errors below \
             go away.\n\n\
             --- SPECIFIC COMPILE ERRORS (cargo check, --message-format=short) ---\n\
             {errors_block}\n\
             --- END ERRORS ---\n\n\
             Fix each error directly. If an error says \"cannot find type X\" use the correct \
             type name from the AVAILABLE WORKSPACE EXPORTS block. If an error says \"function \
             takes N arguments\" match the trait's declared signature. Do NOT invent. Do NOT \
             rename your way out of an error — fix it at the source."
        );
        match send_dm(http, nexus_base, "hex-coder", "auto_repair", &content).await {
            Ok(()) => {
                tracing::info!(
                    rel_path = %rel_path,
                    error_count = err_count,
                    "auto_repair: dispatched code_patch ask"
                );
                let mut s = state().lock().unwrap();
                s.file_cooldowns.insert(rel_path, now);
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
