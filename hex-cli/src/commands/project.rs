//! Project management commands.
//!
//! `hex project register <path>` — register a project with hex-nexus
//! `hex project unregister <id>` — unregister a project
//! `hex project list`            — list registered projects

use clap::Subcommand;
use colored::Colorize;
use serde_json::json;

use crate::fmt::{pretty_table, truncate as fmt_truncate};
use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum ProjectAction {
    /// Register a project with hex-nexus
    Register {
        /// Path to the project root
        path: String,

        /// Optional project name (defaults to directory name)
        #[arg(short, long)]
        name: Option<String>,
    },
    /// Unregister a project from hex-nexus (keeps files)
    Unregister {
        /// Project ID (from `hex project list`)
        id: String,
    },
    /// Archive a project — unregisters and removes .hex/ config but keeps source files
    Archive {
        /// Project ID (from `hex project list`)
        id: String,

        /// Also remove .claude/ directory
        #[arg(long)]
        remove_claude: bool,
    },
    /// Delete a project — unregisters AND removes all project files from disk
    Delete {
        /// Project ID (from `hex project list`)
        id: String,

        /// Required confirmation flag (this is destructive!)
        #[arg(long)]
        confirm: bool,
    },
    /// List registered projects
    List {
        /// Remove all orphaned entries (path no longer exists on disk)
        #[arg(long)]
        clean: bool,
        /// Also remove scratch entries (/tmp, /examples) when used with --clean
        #[arg(long)]
        scratch: bool,
    },
    /// Full project report: agents → swarms → tasks
    Report {
        /// Project ID, name, or prefix. Defaults to current directory name.
        #[arg(default_value = "")]
        id: String,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: ProjectAction) -> anyhow::Result<()> {
    let client = NexusClient::from_env();
    client.ensure_running().await?;

    match action {
        ProjectAction::Register { path, name } => register(&client, &path, name).await,
        ProjectAction::Unregister { id } => unregister(&client, &id).await,
        ProjectAction::Archive { id, remove_claude } => {
            archive(&client, &id, remove_claude).await
        }
        ProjectAction::Delete { id, confirm } => delete(&client, &id, confirm).await,
        ProjectAction::List { clean, scratch } => list(&client, clean, scratch).await,
        ProjectAction::Report { id, json } => report(&client, &id, json).await,
    }
}

async fn register(client: &NexusClient, path: &str, name: Option<String>) -> anyhow::Result<()> {
    let abs_path = std::path::Path::new(path)
        .canonicalize()
        .unwrap_or_else(|_| std::path::PathBuf::from(path));

    let mut body = json!({ "rootPath": abs_path.display().to_string() });
    if let Some(n) = &name {
        body["name"] = json!(n);
    }

    let resp = client.post("/api/projects/register", &body).await?;

    let id = resp["id"].as_str().unwrap_or("?");
    let proj_name = resp["name"].as_str().unwrap_or("?");

    println!("{} Project registered", "\u{2b21}".cyan());
    println!("  ID:   {}", id);
    println!("  Name: {}", proj_name);
    println!("  Path: {}", abs_path.display());

    Ok(())
}

async fn unregister(client: &NexusClient, id: &str) -> anyhow::Result<()> {
    client.delete(&format!("/api/projects/{}", id)).await?;
    println!("{} Project {} unregistered", "\u{2b21}".cyan(), id);
    Ok(())
}

async fn archive(
    client: &NexusClient,
    id: &str,
    remove_claude: bool,
) -> anyhow::Result<()> {
    // Get project details first so we know the root path
    let resp = client.get("/api/projects").await?;
    let projects = resp["projects"].as_array();
    let project = projects
        .and_then(|list| list.iter().find(|p| p["id"].as_str() == Some(id)));

    let root_path = match project {
        Some(p) => p["rootPath"].as_str().unwrap_or("").to_string(),
        None => anyhow::bail!("Project {} not found. Run `hex project list` to see IDs.", id),
    };
    let project_name = project
        .and_then(|p| p["name"].as_str())
        .unwrap_or("?");

    println!(
        "{} Archiving project {} ({})",
        "\u{2b21}".cyan(),
        project_name.bold(),
        root_path
    );

    // 1. Unregister from nexus
    client.delete(&format!("/api/projects/{}", id)).await?;
    println!("  {} Unregistered from nexus", "\u{2713}".green());

    // 2. Remove .hex/ config directory
    let hex_dir = std::path::Path::new(&root_path).join(".hex");
    if hex_dir.exists() {
        std::fs::remove_dir_all(&hex_dir)?;
        println!("  {} Removed .hex/", "\u{2713}".green());
    }

    // 3. Remove .mcp.json (hex-specific config)
    let mcp_json = std::path::Path::new(&root_path).join(".mcp.json");
    if mcp_json.exists() {
        std::fs::remove_file(&mcp_json)?;
        println!("  {} Removed .mcp.json", "\u{2713}".green());
    }

    // 4. Optionally remove .claude/ directory
    if remove_claude {
        let claude_dir = std::path::Path::new(&root_path).join(".claude");
        if claude_dir.exists() {
            std::fs::remove_dir_all(&claude_dir)?;
            println!("  {} Removed .claude/", "\u{2713}".green());
        }
    }

    println!();
    println!(
        "{} Project {} archived — source files preserved at {}",
        "\u{2b21}".green(),
        project_name.bold(),
        root_path
    );

    Ok(())
}

async fn delete(
    client: &NexusClient,
    id: &str,
    confirm: bool,
) -> anyhow::Result<()> {
    if !confirm {
        anyhow::bail!(
            "This will permanently delete all project files!\n  \
             Re-run with --confirm to proceed:\n  \
             hex project delete {} --confirm",
            id
        );
    }

    // Get project details
    let resp = client.get("/api/projects").await?;
    let projects = resp["projects"].as_array();
    let project = projects
        .and_then(|list| list.iter().find(|p| p["id"].as_str() == Some(id)));

    let root_path = match project {
        Some(p) => p["rootPath"].as_str().unwrap_or("").to_string(),
        None => anyhow::bail!("Project {} not found. Run `hex project list` to see IDs.", id),
    };
    let project_name = project
        .and_then(|p| p["name"].as_str())
        .unwrap_or("?");

    // Safety: refuse to delete system directories
    let path = std::path::Path::new(&root_path);
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let path_str = canon.to_string_lossy();

    if path_str == "/"
        || path_str.starts_with("/System")
        || path_str.starts_with("/usr")
        || path_str.starts_with("/bin")
        || path_str.starts_with("/sbin")
        || path_str.starts_with("/var")
        || path_str == std::env::var("HOME").unwrap_or_default()
    {
        anyhow::bail!(
            "Refusing to delete protected path: {}\n  \
             This looks like a system directory or home folder.",
            root_path
        );
    }

    println!(
        "{} {} Deleting project {} and ALL files at:",
        "\u{26a0}".yellow(),
        "WARNING:".red().bold(),
        project_name.bold()
    );
    println!("  {}", root_path.red());
    println!();

    // 1. Unregister from nexus
    client.delete(&format!("/api/projects/{}", id)).await?;
    println!("  {} Unregistered from nexus", "\u{2713}".green());

    // 2. Delete entire project directory
    if path.exists() {
        std::fs::remove_dir_all(path)?;
        println!("  {} Deleted {}", "\u{2713}".green(), root_path);
    } else {
        println!(
            "  {} Directory already gone: {}",
            "\u{2717}".yellow(),
            root_path
        );
    }

    println!();
    println!(
        "{} Project {} permanently deleted",
        "\u{2b21}".red(),
        project_name.bold()
    );

    Ok(())
}

async fn list(client: &NexusClient, clean: bool, scratch: bool) -> anyhow::Result<()> {
    let resp = client.get("/api/projects").await?;

    let projects = match resp["projects"].as_array() {
        Some(list) if !list.is_empty() => list.clone(),
        _ => {
            println!("{} No projects registered", "\u{2b21}".cyan());
            println!("  Register one with: hex project register /path/to/project");
            return Ok(());
        }
    };

    // --clean [--scratch]: remove orphaned (and optionally scratch) entries, then re-fetch
    if clean {
        let to_remove: Vec<_> = projects
            .iter()
            .filter(|p| {
                let status = p["status"].as_str().unwrap_or("");
                status == "orphaned" || (scratch && status == "scratch")
            })
            .collect();
        if to_remove.is_empty() {
            let what = if scratch { "orphaned or scratch" } else { "orphaned" };
            println!("{} No {} projects found", "\u{2b21}".cyan(), what);
        } else {
            for p in &to_remove {
                let id = p["id"].as_str().unwrap_or("?");
                let path = p["rootPath"].as_str().unwrap_or("?");
                let status = p["status"].as_str().unwrap_or("?");
                match client.delete(&format!("/api/projects/{}", id)).await {
                    Ok(_) => println!("{} Removed {} [{}]: {}", "\u{2717}".red(), status, id, path),
                    Err(e) => println!("{} Failed to remove {}: {}", "\u{26a0}".yellow(), id, e),
                }
            }
            println!();
        }
        // Re-fetch after cleanup
        return Box::pin(list(client, false, false)).await;
    }

    let rows: Vec<Vec<String>> = projects
        .iter()
        .map(|p| {
            let id = fmt_truncate(p["id"].as_str().unwrap_or("?"), 36);
            let name = fmt_truncate(p["name"].as_str().unwrap_or("?"), 40).bold().to_string();
            let root = fmt_truncate(p["rootPath"].as_str().unwrap_or("?"), 50);
            let status = status_badge(p["status"].as_str().unwrap_or("unknown"));
            vec![id, name, root, status]
        })
        .collect();

    println!("{}", pretty_table(&["ID", "Name", "Path", "Status"], &rows));

    // Summary counts by status
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for p in &projects {
        *counts.entry(p["status"].as_str().unwrap_or("unknown")).or_insert(0) += 1;
    }
    let summary: Vec<String> = ["active", "recent", "idle", "scratch", "untracked", "orphaned"]
        .iter()
        .filter_map(|s| counts.get(s).map(|n| format!("{} {}", n, s)))
        .collect();
    println!("  {} project{}  ({})", projects.len(),
        if projects.len() == 1 { "" } else { "s" },
        if summary.is_empty() { "none".to_string() } else { summary.join(" · ") }
    );
    if counts.get("orphaned").copied().unwrap_or(0) > 0 {
        println!("  {} Run `hex project list --clean` to remove orphaned entries", "\u{26a0}".yellow());
    }

    Ok(())
}

fn status_badge(status: &str) -> String {
    match status {
        "active"    => "● active".green().to_string(),
        "recent"    => "● recent".cyan().to_string(),
        "idle"      => "○ idle".dimmed().to_string(),
        "scratch"   => "◌ scratch".dimmed().to_string(),
        "untracked" => "? untracked".yellow().to_string(),
        "orphaned"  => "✗ orphaned".red().to_string(),
        other       => other.to_string(),
    }
}

/// Query SpacetimeDB (via nexus) for the project whose rootPath contains CWD.
/// Returns the project ID if an unambiguous match is found.
async fn resolve_project_id_from_cwd(client: &NexusClient) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let resp = client.get("/api/projects").await.ok()?;
    let projects = resp["projects"].as_array()?;

    // Prefer exact rootPath match, then prefix match (CWD is inside a registered project).
    let mut best: Option<(usize, &str)> = None; // (path_len, id)
    for p in projects {
        let root = p["rootPath"].as_str()?;
        let root_path = std::path::Path::new(root);
        if cwd == root_path || cwd.starts_with(root_path) {
            let len = root.len();
            if best.is_none_or(|(prev_len, _)| len > prev_len) {
                best = Some((len, p["id"].as_str()?));
            }
        }
    }
    best.map(|(_, id)| id.to_string())
}

async fn report(client: &NexusClient, id: &str, json_output: bool) -> anyhow::Result<()> {
    // Resolve project ID: explicit arg → rootPath match in SpacetimeDB → current dir name
    let resolved_id = if !id.is_empty() {
        id.to_string()
    } else {
        resolve_project_id_from_cwd(client).await
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                    .unwrap_or_default()
            })
    };

    let resp = client
        .get(&format!("/api/projects/{}/report", resolved_id))
        .await
        .map_err(|e| {
            anyhow::anyhow!("{}\n  Tip: run `hex project list` to see registered project IDs", e)
        })?;

    if json_output {
        println!("{}", serde_json::to_string_pretty(&resp)?);
        return Ok(());
    }

    let w = 65usize;
    let rule = "─".repeat(w);
    let top  = "═".repeat(w);

    // ── Header ───────────────────────────────────────────
    let proj    = &resp["project"];
    let summary = &resp["summary"];
    let arts    = &resp["artifacts"];
    let name    = proj["name"].as_str().unwrap_or("?");
    let root    = proj["rootPath"].as_str().unwrap_or("?");
    let proj_id = proj["id"].as_str().unwrap_or("?");

    println!();
    println!("{}", top.cyan());
    println!("  {} {}", "\u{2b21}".cyan(), name.bold());
    println!("{}", top.cyan());
    println!("  {}  {}", "id  ".dimmed(), proj_id.dimmed());
    println!("  {}  {}", "path".dimmed(), root.dimmed());

    // ── Architecture Health ──────────────────────────────
    // Run `hex analyze <path> --json` in a subprocess so we reuse the full
    // analysis logic without coupling the report to analyze internals.
    let arch = run_analyze_json(root).await;
    println!();
    println!("{}", format!("── Architecture Health {}", "─".repeat(w.saturating_sub(23))).dimmed());
    match &arch {
        Some(a) => {
            let score = a["score"].as_u64().unwrap_or(0);
            let violations: Vec<_> = a["violations"].as_array()
                .into_iter().flatten()
                .chain(a["rust_violations"].as_array().into_iter().flatten())
                .collect();
            let grade = score_to_grade(score);
            let grade_colored = match grade {
                "A+" | "A" => grade.green().bold().to_string(),
                "B"        => grade.yellow().bold().to_string(),
                _          => grade.red().bold().to_string(),
            };
            println!("  {} Grade {}  score {}/100  {} violations",
                "\u{2b21}".cyan(),
                grade_colored,
                score,
                if violations.is_empty() {
                    "0".green().to_string()
                } else {
                    violations.len().to_string().red().to_string()
                }
            );
            // Show first 5 violations if any
            for v in violations.iter().take(5) {
                let msg = v["message"].as_str()
                    .or_else(|| v.as_str())
                    .unwrap_or("violation");
                println!("    {} {}", "✗".red(), fmt_truncate(msg, 60));
            }
            if violations.len() > 5 {
                println!("    {} … and {} more violations", "✗".red(), violations.len() - 5);
            }
        }
        None => {
            println!("  {}", "analyze unavailable (run `hex analyze .` manually)".dimmed());
        }
    }

    // ── Development Artifacts ────────────────────────────
    let adrs      = &arts["adrs"];
    let workplans = &arts["workplans"];
    let specs     = &arts["specs"];

    let adr_total      = adrs["total"].as_u64().unwrap_or(0);
    let adr_accepted   = adrs["accepted"].as_u64().unwrap_or(0);
    let adr_proposed   = adrs["proposed"].as_u64().unwrap_or(0);
    let adr_deprecated = adrs["deprecated"].as_u64().unwrap_or(0);
    let wp_total       = workplans["total"].as_u64().unwrap_or(0);
    let wp_active      = workplans["active"].as_u64().unwrap_or(0);
    let spec_total     = specs["total"].as_u64().unwrap_or(0);

    if adr_total > 0 || wp_total > 0 || spec_total > 0 {
        println!();
        println!("{}", format!("── Development Artifacts {}", "─".repeat(w.saturating_sub(25))).dimmed());

        if adr_total > 0 {
            let adr_detail = [
                if adr_accepted   > 0 { format!("{} accepted",   adr_accepted)   } else { String::new() },
                if adr_proposed   > 0 { format!("{} proposed",   adr_proposed)   } else { String::new() },
                if adr_deprecated > 0 { format!("{} deprecated", adr_deprecated) } else { String::new() },
            ].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("  ");
            println!("  {} ADRs       {}", format!("{:>3}", adr_total).white().bold(), adr_detail.dimmed());
        }
        if wp_total > 0 {
            let active_str = if wp_active > 0 {
                format!("  ({} active)", wp_active).yellow().to_string()
            } else {
                "  (all done)".dimmed().to_string()
            };
            println!("  {} Workplans {}{}", format!("{:>3}", wp_total).white().bold(), String::new(), active_str);
        }
        if spec_total > 0 {
            println!("  {} Specs", format!("{:>3}", spec_total).white().bold());
        }
    }

    // ── Active Workplans ─────────────────────────────────
    if let Some(wp_list) = workplans["list"].as_array() {
        let active_wps: Vec<_> = wp_list.iter()
            .filter(|w| w["active"].as_bool().unwrap_or(false))
            .collect();
        if !active_wps.is_empty() {
            println!();
            println!("{}", format!("── Active Workplans {}", "─".repeat(w.saturating_sub(20))).dimmed());
            for wp in &active_wps {
                let feat    = wp["feature"].as_str().unwrap_or("?");
                let total   = wp["totalPhases"].as_u64().unwrap_or(0);
                let done    = wp["donePhases"].as_u64().unwrap_or(0);
                let pending = total.saturating_sub(done);
                let progress = if total > 0 {
                    format!("{}/{} phases done", done, total)
                } else {
                    "no phases".to_string()
                };
                let pending_str = if pending > 0 {
                    format!("  ({} remaining)", pending).yellow().to_string()
                } else {
                    String::new()
                };
                println!("  {} {}  {}{}",
                    "⟳".yellow(),
                    fmt_truncate(feat, 44).bold(),
                    progress.dimmed(),
                    pending_str,
                );
            }
        }
    }

    // ── Swarms ───────────────────────────────────────────
    let swarms = resp["swarms"].as_array();
    if let Some(swarms) = swarms.filter(|s| !s.is_empty()) {
        let stale_count     = swarms.iter().filter(|s| s["status"].as_str() == Some("stale")).count();
        let completed_count = swarms.iter().filter(|s| s["status"].as_str() == Some("completed")).count();
        let show_swarms: Vec<_> = swarms.iter()
            .filter(|s| matches!(s["status"].as_str(), Some("active") | Some("failed")))
            .collect();

        println!();
        println!("{}", format!("── Swarms {}", "─".repeat(w.saturating_sub(10))).dimmed());
        for swarm in &show_swarms {
            let sname   = swarm["name"].as_str().unwrap_or("?");
            let status  = swarm["status"].as_str().unwrap_or("?");
            let st      = &swarm["tasks"];
            let s_total = st["total"].as_u64().unwrap_or(0);
            let s_done  = st["completed"].as_u64().unwrap_or(0);
            let s_fail  = st["failed"].as_u64().unwrap_or(0);
            let s_prog  = st["inProgress"].as_u64().unwrap_or(0);

            let (icon, name_colored) = match status {
                "active"    => ("⟳".yellow().to_string(), sname.bold().yellow().to_string()),
                "failed"    => ("✗".red().to_string(),    sname.bold().red().to_string()),
                "completed" => ("✓".green().to_string(),  sname.dimmed().to_string()),
                _           => ("○".normal().to_string(), sname.to_string()),
            };

            let task_bar = if s_total > 0 {
                let bar_width = 16usize;
                let filled = ((s_done as f64 / s_total as f64) * bar_width as f64) as usize;
                let bar = format!("[{}{}]",
                    "█".repeat(filled).green(),
                    "░".repeat(bar_width.saturating_sub(filled)).dimmed()
                );
                format!("  {} {}/{}", bar, s_done, s_total)
            } else {
                "  (no tasks)".dimmed().to_string()
            };

            // Only show in-progress/failed counts for non-completed swarms —
            // completed swarms may still have tasks in in_progress state if they were purged.
            let extras = if status != "completed" {
                [
                    if s_prog > 0 { format!("{} running", s_prog).yellow().to_string() } else { String::new() },
                    if s_fail > 0 { format!("{} failed",  s_fail).red().to_string()    } else { String::new() },
                ].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("  ")
            } else {
                String::new()
            };

            print!("  {} {}  {}", icon, name_colored, task_bar);
            if !extras.is_empty() { print!("  {}", extras); }
            println!();

            // For active/failed swarms: show task detail (failed + in-progress only)
            if status == "active" || status == "failed" {
                if let Some(task_list) = swarm["taskList"].as_array() {
                    let show: Vec<_> = task_list.iter()
                        .filter(|t| matches!(t["status"].as_str(), Some("in_progress") | Some("failed")))
                        .collect();
                    for task in &show {
                        let title   = task["title"].as_str().unwrap_or("?");
                        let tstatus = task["status"].as_str().unwrap_or("?");
                        let icon    = task_status_icon(tstatus);
                        let display = if let Some(pos) = title.rfind(" [retry ") {
                            format!("{}…{}", fmt_truncate(&title[..pos], 40), &title[pos..])
                        } else {
                            fmt_truncate(title, 50)
                        };
                        println!("      {} {}  {}", icon, display, colorize_task_status(tstatus));
                    }
                }
            }
        }
        if show_swarms.is_empty() {
            if completed_count > 0 {
                println!("  {} {} completed — use `hex swarm list --all` to view",
                    "○".dimmed(), completed_count.to_string().dimmed());
            }
        } else if completed_count > 0 {
            println!("  {} {} more completed", "○".dimmed(), completed_count.to_string().dimmed());
        }
        if stale_count > 0 {
            println!("  {} {} zombie swarms hidden (agent died mid-run — run `hex swarm cleanup --apply` to remove)",
                "⚠".yellow(), stale_count);
        }
    } else {
        println!();
        println!("  {}", "No swarms yet.".dimmed());
    }

    // ── Agents ───────────────────────────────────────────
    let agents = resp["agents"].as_array();
    let live_agents: Vec<_> = agents.map(|a| {
        a.iter().filter(|ag| !ag["historical"].as_bool().unwrap_or(false)).collect()
    }).unwrap_or_default();

    if !live_agents.is_empty() {
        println!();
        println!("{}", format!("── Agents {}", "─".repeat(w.saturating_sub(10))).dimmed());
        for a in &live_agents {
            let aname  = a["name"].as_str().unwrap_or("?");
            let status = a["status"].as_str().unwrap_or("?");
            let model  = a["model"].as_str().unwrap_or("");
            let model_str = if model.is_empty() { String::new() } else { format!("  {}", model.dimmed()) };
            println!("  {} {}  {}{}", "\u{25cf}".cyan(), aname.bold(), colorize_agent_status(status), model_str);
        }
    }

    // ── Footer ────────────────────────────────────────────
    let tasks_total  = summary["tasksTotal"].as_u64().unwrap_or(0);
    let tasks_done   = summary["tasksCompleted"].as_u64().unwrap_or(0);
    let tasks_failed = summary["tasksFailed"].as_u64().unwrap_or(0);
    let swarm_count  = summary["swarmCount"].as_u64().unwrap_or(0);

    println!();
    println!("{}", rule.dimmed());
    let footer_parts = [
        if swarm_count > 0 { format!("{} swarms", swarm_count) } else { String::new() },
        if tasks_total  > 0 { format!("{}/{} tasks done", tasks_done, tasks_total) } else { String::new() },
        if tasks_failed > 0 { format!("{} failed", tasks_failed).red().to_string()  } else { String::new() },
    ].iter().filter(|s| !s.is_empty()).cloned().collect::<Vec<_>>().join("  ·  ");
    if !footer_parts.is_empty() {
        println!("  {}", footer_parts.dimmed());
    }
    println!();
    Ok(())
}

fn task_status_icon(status: &str) -> &'static str {
    match status {
        "completed"   => "✓",
        "failed"      => "✗",
        "in_progress" => "⟳",
        _             => "○",
    }
}

fn colorize_agent_status(s: &str) -> String {
    match s {
        "online"   | "active"   => s.green().to_string(),
        "inactive" | "stale"    => s.yellow().to_string(),
        "dead"     | "offline"  => s.red().to_string(),
        _                       => s.dimmed().to_string(),
    }
}


fn colorize_task_status(s: &str) -> String {
    match s {
        "completed"   => s.green().to_string(),
        "failed"      => s.red().to_string(),
        "in_progress" => s.yellow().to_string(),
        _             => s.dimmed().to_string(),
    }
}

async fn run_analyze_json(path: &str) -> Option<serde_json::Value> {
    let exe = std::env::current_exe().ok()?;
    let out = tokio::process::Command::new(exe)
        .args(["analyze", path, "--json"])
        .output()
        .await
        .ok()?;
    if !out.status.success() { return None; }
    serde_json::from_slice(&out.stdout).ok()
}

fn score_to_grade(score: u64) -> &'static str {
    match score {
        100       => "A+",
        90..=99   => "A",
        80..=89   => "B",
        70..=79   => "C",
        60..=69   => "D",
        _         => "F",
    }
}
