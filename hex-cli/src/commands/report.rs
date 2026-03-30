//! `hex report` — developer audit report for hex dev sessions.
//!
//! Assembles a complete trace from ask → ADR → workplan → swarm → tasks → code → validation
//! using session files, SpacetimeDB, workplan JSON, and git history.

use anyhow::{bail, Result};
use chrono::DateTime;
use clap::Subcommand;
use colored::Colorize;
use serde_json::Value;
use std::path::Path;
use tabled::Tabled;

use crate::fmt::{HexTable, status_badge, truncate as fmt_truncate};
use crate::nexus_client::NexusClient;
use crate::session::{DevSession, DevSessionSummary, SessionStatus};

#[derive(Subcommand)]
pub enum ReportAction {
    /// Generate audit report for a specific session
    Show {
        /// Session ID (or prefix)
        id: String,

        /// Output as JSON instead of formatted text
        #[arg(long)]
        json: bool,
    },
    /// Generate audit report for the most recent completed session
    Latest {
        /// Output as JSON instead of formatted text
        #[arg(long)]
        json: bool,
    },
    /// List all sessions available for reporting
    List,
}

pub async fn run(action: ReportAction) -> Result<()> {
    match action {
        ReportAction::Show { id, json } => show_report(&id, json).await,
        ReportAction::Latest { json } => show_latest(json).await,
        ReportAction::List => list_sessions().await,
    }
}

async fn list_sessions() -> Result<()> {
    let sessions = DevSession::list_all()?;
    if sessions.is_empty() {
        println!("No dev sessions found. Run `hex dev start` to create one.");
        return Ok(());
    }
    println!("{}", "hex dev — Sessions".cyan().bold());
    println!();

    #[derive(Tabled)]
    struct SessionRow {
        #[tabled(rename = "ID")]
        id: String,
        #[tabled(rename = "Status")]
        status: String,
        #[tabled(rename = "Phase")]
        phase: String,
        #[tabled(rename = "Cost")]
        cost: String,
        #[tabled(rename = "Feature")]
        feature: String,
    }

    let rows: Vec<SessionRow> = sessions
        .iter()
        .map(|s| SessionRow {
            id: s.id.clone(),
            status: status_badge(&format!("{}", s.status)),
            phase: format!("{}", s.current_phase),
            cost: format!("${:.4}", s.total_cost_usd),
            feature: fmt_truncate(&s.feature_description, 50),
        })
        .collect();

    println!("{}", HexTable::render(&rows));
    println!();
    println!("  Run `hex report show <id>` for a full audit report.");
    Ok(())
}

async fn show_latest(json: bool) -> Result<()> {
    let mut sessions = DevSession::list_all()?;
    // Sort by updated_at descending, prefer sessions with actual work (cost > 0)
    sessions.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    let latest = sessions
        .iter()
        .find(|s| matches!(s.status, SessionStatus::Completed));
    match latest {
        Some(s) => show_report(&s.id, json).await,
        None => {
            println!("No completed sessions found.");
            Ok(())
        }
    }
}

async fn show_report(id: &str, json_output: bool) -> Result<()> {
    // Find session by ID or prefix
    let session = find_session(id)?;
    let client = NexusClient::from_env();

    // Resolve project_id: session field first, fallback to .hex/project.json
    let project_id = session.project_id.clone().or_else(|| {
        let project_json = Path::new(".hex/project.json");
        if project_json.exists() {
            std::fs::read_to_string(project_json)
                .ok()
                .and_then(|c| serde_json::from_str::<Value>(&c).ok())
                .and_then(|v| v["id"].as_str().map(|s| s.to_string()))
        } else {
            None
        }
    });

    // Gather all data
    let adr_info = gather_adr_info(&session);
    let workplan_info = gather_workplan_info(&session);
    let swarm_info = gather_swarm_info(&session, &client).await;
    let git_info = gather_git_info(&session);
    let quality_gates = gather_quality_gates(&session, &client).await;

    if json_output {
        print_json_report(&session, &project_id, &adr_info, &workplan_info, &swarm_info, &git_info, &quality_gates);
        return Ok(());
    }

    // ── Header ──────────────────────────────────────────────
    println!();
    println!("{}", "═".repeat(65).cyan());
    println!("  {}", "hex dev — Audit Report".cyan().bold());
    println!("{}", "═".repeat(65).cyan());
    println!();

    // ── Ask ─────────────────────────────────────────────────
    println!(
        "  {}  \"{}\"",
        "Ask:".white().bold(),
        session.feature_description.yellow()
    );
    println!("  {}  {}", "Session:".white().bold(), session.id.dimmed());
    println!(
        "  {}  {}",
        "Status: ".white().bold(),
        colorize_status(&session.status)
    );
    if let Some(ref pid) = project_id {
        println!("  {}  {}", "Project:".white().bold(), pid.dimmed());
    }
    if let Some(ref agent_id) = session.agent_id {
        println!("  {}  {}", "Agent:  ".white().bold(), agent_id.dimmed());
    }
    println!("  {}  {}", "Started:".white().bold(), session.created_at);
    println!("  {}  {}", "Ended:  ".white().bold(), session.updated_at);

    // Duration
    if let (Ok(start), Ok(end)) = (
        DateTime::parse_from_rfc3339(&session.created_at),
        DateTime::parse_from_rfc3339(&session.updated_at),
    ) {
        let dur = end.signed_duration_since(start);
        println!(
            "  {} {}s",
            "Duration:".white().bold(),
            dur.num_seconds()
        );
    }

    println!(
        "  {}  ${:.4} ({} tokens)",
        "Cost:   ".white().bold(),
        session.total_cost_usd,
        format_tokens(session.total_tokens)
    );

    // ── Phase 1: ADR ────────────────────────────────────────
    println!();
    println!("{}", "── Phase 1: ADR ──────────────────────────────────────────────".dimmed());
    match &adr_info {
        Some(info) => {
            println!("  File:   {}", info.path.green());
            println!("  Lines:  {}", info.lines);
            println!("  Size:   {} bytes", info.size);
            if !info.title.is_empty() {
                println!("  Title:  {}", info.title);
            }
        }
        None => println!("  {}", "No ADR generated (skipped or failed)".dimmed()),
    }

    // ── Phase 2: Workplan ───────────────────────────────────
    println!();
    println!("{}", "── Phase 2: Workplan ─────────────────────────────────────────".dimmed());
    match &workplan_info {
        Some(info) => {
            println!("  File:   {}", info.path.green());
            println!("  Title:  {}", info.title);
            println!("  Steps:  {}", info.total_steps);
            println!("  Tiers:  {}", info.tier_summary);
            println!();

            #[derive(Tabled)]
            struct StepRow {
                #[tabled(rename = "Step")]
                id: String,
                #[tabled(rename = "Tier")]
                tier: String,
                #[tabled(rename = "Description")]
                description: String,
                #[tabled(rename = "Adapter")]
                adapter: String,
            }

            let step_rows: Vec<StepRow> = info.steps.iter().map(|step| StepRow {
                id: step.id.clone(),
                tier: format!("T{}", step.tier),
                description: truncate(&step.description, 50),
                adapter: step.adapter.clone(),
            }).collect();

            println!("{}", HexTable::render(&step_rows));
        }
        None => println!("  {}", "No workplan generated (skipped or failed)".dimmed()),
    }

    // ── Phase 3: Swarm ──────────────────────────────────────
    println!();
    println!("{}", "── Phase 3: Swarm ────────────────────────────────────────────".dimmed());
    match &swarm_info {
        Some(info) => {
            println!("  Swarm:    {}", info.id.dimmed());
            println!("  Name:     {}", info.name);
            println!("  Topology: {}", info.topology);
            println!("  Status:   {}", colorize_swarm_status(&info.status));
            println!("  Tasks:    {}/{} completed", info.tasks_completed, info.tasks_total);
            if !info.tasks.is_empty() {
                println!();

                #[derive(Tabled)]
                struct TaskRow {
                    #[tabled(rename = "Task")]
                    id: String,
                    #[tabled(rename = "Status")]
                    status: String,
                    #[tabled(rename = "Title")]
                    title: String,
                }

                let task_rows: Vec<TaskRow> = info.tasks.iter().map(|task| TaskRow {
                    id: task.id[..8.min(task.id.len())].to_string(),
                    status: status_badge(&task.status),
                    title: truncate(&task.title, 50),
                }).collect();

                println!("{}", HexTable::render(&task_rows));
            }
        }
        None => println!("  {}", "No swarm created (skipped or failed)".dimmed()),
    }

    // ── Phase 4: Code Generation ────────────────────────────
    println!();
    println!("{}", "── Phase 4: Code Generation ──────────────────────────────────".dimmed());
    if !session.completed_steps.is_empty() {
        // Interactive gate path — steps tracked individually
        println!("  Steps completed: {}", session.completed_steps.len());
        println!(
            "  Steps:  {}",
            session.completed_steps.join(", ")
        );
    } else if session.quality_result.as_ref().map(|q| q.compile_pass).unwrap_or(false) {
        // Supervisor auto path — steps not tracked individually but code compiled
        let lang = session.quality_result.as_ref()
            .map(|q| q.compile_language.as_str())
            .unwrap_or("unknown");
        let iters = session.quality_result.as_ref()
            .map(|q| q.iterations)
            .unwrap_or(0);
        println!("  Mode:       supervisor (auto)");
        println!("  Language:   {}", lang);
        println!("  Iterations: {}", iters);
        println!("  Compile:    {}", "PASS".green());
    } else {
        println!("  {}", "No code generated (skipped or failed)".dimmed());
    }

    // ── Phase 5: Quality Gate ────────────────────────────────
    println!();
    println!("{}", "── Phase 5: Quality Gate ─────────────────────────────────────".dimmed());
    if let Some(ref qr) = session.quality_result {
        let grade_colored = match qr.grade.as_str() {
            "A" => qr.grade.green().bold().to_string(),
            "B" => qr.grade.green().to_string(),
            "C" => qr.grade.yellow().to_string(),
            "D" => qr.grade.red().to_string(),
            _ => qr.grade.red().bold().to_string(),
        };
        println!("  Grade:       {} ({}/100)", grade_colored, qr.score);
        println!("  Iterations:  {}", qr.iterations);
        let compile_status = if qr.compile_pass {
            "PASS".green().to_string()
        } else {
            "FAIL".red().to_string()
        };
        println!("  Compile:     {} ({})", compile_status, qr.compile_language);
        let test_status = if qr.test_pass {
            format!("{}/{} passing", qr.tests_passed, qr.tests_passed + qr.tests_failed).green().to_string()
        } else {
            format!("{}/{} passing", qr.tests_passed, qr.tests_passed + qr.tests_failed).red().to_string()
        };
        println!("  Tests:       {}", test_status);
        let violations_str = if qr.violations_found == 0 && qr.violations_fixed == 0 {
            "0".green().to_string()
        } else if qr.violations_found == 0 {
            format!("0 ({} fixed)", qr.violations_fixed).green().to_string()
        } else {
            format!("{} ({} fixed)", qr.violations_found, qr.violations_fixed).yellow().to_string()
        };
        println!("  Violations:  {}", violations_str);
        if qr.fix_cost_usd > 0.0 || qr.fix_tokens > 0 {
            println!(
                "  Fix Cost:    ${:.3} ({} tokens)",
                qr.fix_cost_usd, format_tokens(qr.fix_tokens)
            );
        }
    } else {
        match session.current_phase {
            crate::session::PipelinePhase::Validate | crate::session::PipelinePhase::Commit => {
                println!("  Status: {}", "PASS".green());
            }
            _ => {
                println!("  Status: {}", "Not reached".dimmed());
            }
        }
    }

    // ── Quality Gates (SpacetimeDB) ──────────────────────────
    if !quality_gates.is_empty() {
        println!();
        println!("{}", "── Quality Gates (SpacetimeDB) ───────────────────────────────".dimmed());
        for qg in &quality_gates {
            let status_colored = match qg.status.as_str() {
                "pass" => "PASS".green().to_string(),
                "fail" => "FAIL".red().to_string(),
                "running" => "RUNNING".yellow().to_string(),
                _ => qg.status.to_uppercase().dimmed().to_string(),
            };
            let score_str = match (qg.score, qg.grade.as_deref()) {
                (Some(s), Some(g)) => format!("Score {}/{}", s, g),
                (Some(s), None) => format!("Score {}", s),
                _ => String::new(),
            };
            let iter_str = if qg.iterations > 1 {
                format!("({} iterations", qg.iterations)
            } else {
                format!("({} iteration", qg.iterations)
            };
            let fix_count = qg.fixes.len();
            let fix_suffix = if fix_count > 0 {
                format!(", {} fix{})", fix_count, if fix_count == 1 { "" } else { "es" })
            } else {
                ")".to_string()
            };
            println!(
                "  Tier {}:  {}  {}  {}{}",
                qg.tier, status_colored, score_str, iter_str, fix_suffix
            );
            for fix in &qg.fixes {
                let cost_str = if fix.cost_usd > 0.0 {
                    format!(", ${:.3}", fix.cost_usd)
                } else {
                    String::new()
                };
                println!(
                    "    Fix: {} — {} ({}{})",
                    fix.file, fix.issue, fix.model, cost_str
                );
            }
        }
    }

    // ── Agent Reports ──────────────────────────────────────────
    // Extract per-agent tool calls logged by the supervisor (phase starts with "agent-")
    let agent_calls: Vec<_> = session.tool_calls.iter()
        .filter(|c| c.phase.starts_with("agent-"))
        .collect();
    if !agent_calls.is_empty() {
        println!();
        println!("{}", "── Agent Reports ─────────────────────────────────────────────".dimmed());

        #[derive(Tabled)]
        struct AgentRow {
            #[tabled(rename = "Agent")]
            agent: String,
            #[tabled(rename = "Status")]
            status: String,
            #[tabled(rename = "Model")]
            model: String,
            #[tabled(rename = "Tokens")]
            tokens: String,
            #[tabled(rename = "Context")]
            context: String,
            #[tabled(rename = "Time")]
            time: String,
            #[tabled(rename = "Cost")]
            cost: String,
            #[tabled(rename = "Objective")]
            objective: String,
        }

        let agent_rows: Vec<AgentRow> = agent_calls.iter().map(|call| {
            let role = call.phase.strip_prefix("agent-").unwrap_or(&call.phase);
            let model = call.model.as_deref().unwrap_or("—");
            let model_short = if model.len() > 18 { &model[model.len()-18..] } else { model };
            AgentRow {
                agent: role.to_string(),
                status: status_badge(&call.status),
                model: model_short.to_string(),
                tokens: call.tokens.map(|t| format_tokens(t)).unwrap_or_else(|| "—".into()),
                context: call.input_tokens.map(|t| format_tokens(t)).unwrap_or_else(|| "—".into()),
                time: format!("{:.1}s", call.duration_ms as f64 / 1000.0),
                cost: call.cost_usd.map(|c| format!("${:.4}", c)).unwrap_or_else(|| "—".into()),
                objective: truncate(call.detail.as_deref().unwrap_or("—"), 35),
            }
        }).collect();

        println!("{}", HexTable::render(&agent_rows));
    }

    // ── Git Changes ─────────────────────────────────────────
    println!();
    println!("{}", "── Files Changed ─────────────────────────────────────────────".dimmed());
    match &git_info {
        Some(info) => {
            if !info.created.is_empty() {
                println!("  {} ({}):", "Created".green().bold(), info.created.len());
                for f in &info.created {
                    println!("    {} {}", "+".green(), f);
                }
            }
            if !info.modified.is_empty() {
                println!("  {} ({}):", "Modified".yellow().bold(), info.modified.len());
                for f in &info.modified {
                    println!("    {} {}", "~".yellow(), f);
                }
            }
            if !info.deleted.is_empty() {
                println!("  {} ({}):", "Deleted".red().bold(), info.deleted.len());
                for f in &info.deleted {
                    println!("    {} {}", "-".red(), f);
                }
            }
            if info.created.is_empty() && info.modified.is_empty() && info.deleted.is_empty() {
                println!("  {}", "No file changes detected".dimmed());
            }
            println!();
            println!(
                "  Total: {} created, {} modified, {} deleted",
                info.created.len(),
                info.modified.len(),
                info.deleted.len()
            );
        }
        None => println!("  {}", "Git info unavailable".dimmed()),
    }

    // ── Tools Called ─────────────────────────────────────────
    println!();
    println!("{}", "── Tools Called ──────────────────────────────────────────────".dimmed());
    if session.tool_calls.is_empty() {
        println!("  {}", "No tool calls recorded (session predates tracking or ran in TUI mode)".dimmed());
    } else {
        #[derive(Tabled)]
        struct ToolCallRow {
            #[tabled(rename = "Time")]
            timestamp: String,
            #[tabled(rename = "Phase")]
            phase: String,
            #[tabled(rename = "Tool")]
            tool: String,
            #[tabled(rename = "Model")]
            model: String,
            #[tabled(rename = "Tokens")]
            tokens: String,
            #[tabled(rename = "Context")]
            context: String,
            #[tabled(rename = "Cost")]
            cost: String,
            #[tabled(rename = "Duration")]
            duration: String,
            #[tabled(rename = "Status")]
            status: String,
        }

        let tool_rows: Vec<ToolCallRow> = session.tool_calls.iter().map(|call| {
            let ts = call.timestamp.get(11..19).unwrap_or(&call.timestamp).to_string();
            let model = call.model.as_deref().unwrap_or("—");
            let model_short = if model.len() > 20 {
                &model[model.len()-20..]
            } else {
                model
            };
            ToolCallRow {
                timestamp: ts,
                phase: call.phase.clone(),
                tool: truncate(&call.tool, 30),
                model: model_short.to_string(),
                tokens: call.tokens.map(|t| format_tokens(t)).unwrap_or_else(|| "—".into()),
                context: call.input_tokens.map(|t| format_tokens(t)).unwrap_or_else(|| "—".into()),
                cost: call.cost_usd.map(|c| format!("${:.4}", c)).unwrap_or_else(|| "—".into()),
                duration: format!("{:.1}s", call.duration_ms as f64 / 1000.0),
                status: status_badge(&call.status),
            }
        }).collect();

        println!("{}", HexTable::render(&tool_rows));
        println!();
        let ok_count = session.tool_calls.iter().filter(|c| c.status == "ok").count();
        let err_count = session.tool_calls.iter().filter(|c| c.status == "error").count();
        let retry_count = session.tool_calls.iter().filter(|c| c.status == "retry").count();
        println!(
            "  Total: {} calls ({} ok, {} errors, {} retries)",
            session.tool_calls.len(), ok_count, err_count, retry_count
        );
    }

    // ── Models Used ──────────────────────────────────────────
    {
        use std::collections::HashMap;
        let mut model_stats: HashMap<String, (u64, u64, f64)> = HashMap::new(); // model → (tokens, ctx, cost)
        for call in &session.tool_calls {
            if let Some(ref m) = call.model {
                let entry = model_stats.entry(m.clone()).or_insert((0, 0, 0.0));
                entry.0 += call.tokens.unwrap_or(0);
                entry.1 += call.input_tokens.unwrap_or(0);
                entry.2 += call.cost_usd.unwrap_or(0.0);
            }
        }
        if !model_stats.is_empty() {
            println!();
            println!("{}", "── Models Used ───────────────────────────────────────────────".dimmed());

            #[derive(Tabled)]
            struct ModelRow {
                #[tabled(rename = "Model")]
                model: String,
                #[tabled(rename = "Tokens")]
                tokens: String,
                #[tabled(rename = "Context")]
                context: String,
                #[tabled(rename = "Cost")]
                cost: String,
            }

            let mut model_rows: Vec<ModelRow> = model_stats.iter().map(|(model, (tokens, ctx, cost))| {
                ModelRow {
                    model: model.clone(),
                    tokens: format_tokens(*tokens),
                    context: format_tokens(*ctx),
                    cost: format!("${:.4}", cost),
                }
            }).collect();
            model_rows.sort_by(|a, b| b.tokens.cmp(&a.tokens));
            println!("{}", HexTable::render(&model_rows));
        }
    }

    // ── Summary ─────────────────────────────────────────────
    println!();
    println!("{}", "── Summary ───────────────────────────────────────────────────".dimmed());

    let artifact_count = [
        adr_info.as_ref().map(|_| 1).unwrap_or(0),
        workplan_info.as_ref().map(|_| 1).unwrap_or(0),
    ]
    .iter()
    .sum::<usize>()
        + git_info
            .as_ref()
            .map(|g| g.created.len())
            .unwrap_or(0);

    let models: Vec<&str> = session
        .model_selections
        .values()
        .map(|s| s.as_str())
        .collect();

    println!("  Artifacts:  {}", artifact_count);
    if !models.is_empty() {
        println!("  Models:     {}", models.join(", "));
    }
    let total_ctx: u64 = session.tool_calls.iter().filter_map(|c| c.input_tokens).sum();
    let total_out: u64 = session.tool_calls.iter().filter_map(|c| c.output_tokens).sum();
    if total_ctx > 0 || total_out > 0 {
        println!(
            "  Inference:  {} tokens (ctx: {}, out: {}), ${:.4}",
            format_tokens(session.total_tokens),
            format_tokens(total_ctx),
            format_tokens(total_out),
            session.total_cost_usd
        );
    } else {
        println!(
            "  Inference:  {} tokens, ${:.4}",
            format_tokens(session.total_tokens),
            session.total_cost_usd
        );
    }
    if let Some(ref si) = swarm_info {
        println!(
            "  Swarm:      {} tasks ({} completed, {} pending)",
            si.tasks_total, si.tasks_completed, si.tasks_total - si.tasks_completed
        );
    }

    println!();
    println!("{}", "═".repeat(65).cyan());
    println!();

    Ok(())
}

// ── Data gathering ──────────────────────────────────────────────────────

fn find_session(id: &str) -> Result<DevSession> {
    // Try exact match first
    if let Ok(s) = DevSession::load(id) {
        return Ok(s);
    }
    // Try prefix match
    let sessions = DevSession::list_all()?;
    let matches: Vec<&DevSessionSummary> = sessions
        .iter()
        .filter(|s| s.id.starts_with(id))
        .collect();
    match matches.len() {
        0 => bail!("No session found matching '{}'", id),
        1 => DevSession::load(&matches[0].id),
        n => bail!(
            "Ambiguous: {} sessions match '{}'. Provide more characters.",
            n, id
        ),
    }
}

struct AdrInfo {
    path: String,
    lines: usize,
    size: u64,
    title: String,
}

fn gather_adr_info(session: &DevSession) -> Option<AdrInfo> {
    let path = session.adr_path.as_ref()?;
    let p = Path::new(path);
    if !p.exists() {
        return Some(AdrInfo {
            path: path.clone(),
            lines: 0,
            size: 0,
            title: "(file not found)".into(),
        });
    }
    let content = std::fs::read_to_string(p).ok()?;
    let lines = content.lines().count();
    let size = p.metadata().map(|m| m.len()).unwrap_or(0);
    let title = content
        .lines()
        .find(|l| l.starts_with("# "))
        .unwrap_or("")
        .trim_start_matches("# ")
        .to_string();
    Some(AdrInfo {
        path: path.clone(),
        lines,
        size,
        title,
    })
}

struct WorkplanStep {
    id: String,
    description: String,
    adapter: String,
    tier: u32,
}

struct WorkplanInfo {
    path: String,
    title: String,
    total_steps: usize,
    tier_summary: String,
    steps: Vec<WorkplanStep>,
}

fn gather_workplan_info(session: &DevSession) -> Option<WorkplanInfo> {
    let path = session.workplan_path.as_ref()?;
    let content = std::fs::read_to_string(path).ok()?;
    let data: Value = serde_json::from_str(&content).ok()?;

    let title = data["title"].as_str().unwrap_or("").to_string();
    let steps_arr = data["steps"].as_array()?;

    let mut tier_counts: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    let mut steps = Vec::new();

    for s in steps_arr {
        let tier = s["tier"].as_u64().unwrap_or(0) as u32;
        *tier_counts.entry(tier).or_insert(0) += 1;
        steps.push(WorkplanStep {
            id: s["id"].as_str().unwrap_or("?").to_string(),
            description: s["description"].as_str().unwrap_or("").to_string(),
            adapter: s["adapter"].as_str().unwrap_or("").to_string(),
            tier,
        });
    }

    let tier_summary = tier_counts
        .iter()
        .map(|(t, c)| format!("T{}: {}", t, c))
        .collect::<Vec<_>>()
        .join(", ");

    Some(WorkplanInfo {
        path: path.clone(),
        title,
        total_steps: steps.len(),
        tier_summary,
        steps,
    })
}

struct SwarmTask {
    id: String,
    title: String,
    status: String,
}

struct SwarmInfo {
    id: String,
    name: String,
    topology: String,
    status: String,
    tasks_total: usize,
    tasks_completed: usize,
    tasks: Vec<SwarmTask>,
}

async fn gather_swarm_info(session: &DevSession, client: &NexusClient) -> Option<SwarmInfo> {
    let swarm_id = session.swarm_id.as_ref()?;

    // Try active swarms first, then fall back to all swarms (completed swarms
    // are no longer in the active list by the time the report is shown).
    let swarm = {
        let try_find = |data: &serde_json::Value| -> Option<serde_json::Value> {
            data.as_array()?
                .iter()
                .find(|s| s["id"].as_str().unwrap_or("") == swarm_id)
                .cloned()
        };
        // Try direct lookup by ID first (works for active and completed swarms)
        let direct = client.get(&format!("/api/swarms/{}", swarm_id)).await.ok();
        let found = direct.as_ref().and_then(|d| {
            // GET /api/swarms/{id} returns { "swarm": {...}, "tasks": [...] }
            // unwrap to the swarm object if nested
            if d.get("swarm").is_some() {
                d["swarm"].as_object().map(|_| d["swarm"].clone())
            } else {
                Some(d.clone())
            }
        });
        if found.is_some() {
            found
        } else {
            // Fallback: search active swarms list
            let active = client.get("/api/swarms/active").await.ok();
            active.as_ref().and_then(try_find)
        }
    }?;

    // Also fetch the full response again to extract tasks — the direct lookup
    // returns { "swarm": {...}, "tasks": [...] } so tasks are NOT inside swarm.
    let tasks_from_direct = client.get(&format!("/api/swarms/{}", swarm_id)).await.ok();
    let tasks_top_level = tasks_from_direct.as_ref()
        .and_then(|d| d["tasks"].as_array())
        .cloned();

    let name = swarm["name"].as_str().unwrap_or("").to_string();
    let topology = swarm["topology"].as_str().unwrap_or("").to_string();
    let status = swarm["status"].as_str().unwrap_or("").to_string();

    let mut tasks = Vec::new();
    let mut completed = 0;

    // Tasks live at the top-level "tasks" key of GET /api/swarms/{id},
    // not inside the "swarm" sub-object. Fall back to swarm["tasks"] for
    // older server versions that embed them directly.
    let task_iter: Box<dyn Iterator<Item = &serde_json::Value>> =
        if let Some(ref top) = tasks_top_level {
            Box::new(top.iter())
        } else if let Some(embedded) = swarm["tasks"].as_array() {
            Box::new(embedded.iter())
        } else {
            Box::new(std::iter::empty())
        };

    for t in task_iter {
        let task_status = t["status"].as_str().unwrap_or("pending").to_string();
        if task_status == "completed" {
            completed += 1;
        }
        let raw_title = t["title"].as_str().unwrap_or("").to_string();
            // Task titles may be stored as JSON objects like {"description":"..."}
            let title = if raw_title.starts_with('{') {
                serde_json::from_str::<serde_json::Value>(&raw_title)
                    .ok()
                    .and_then(|v| v["description"].as_str().map(|s| s.to_string()))
                    .unwrap_or(raw_title)
            } else {
                raw_title
            };
            tasks.push(SwarmTask {
                id: t["id"].as_str().unwrap_or("").to_string(),
                title,
                status: task_status,
            });
    }

    Some(SwarmInfo {
        id: swarm_id.clone(),
        name,
        topology,
        status,
        tasks_total: tasks.len(),
        tasks_completed: completed,
        tasks,
    })
}

struct QualityGateInfo {
    tier: u32,
    status: String,
    score: Option<u32>,
    grade: Option<String>,
    iterations: u32,
    fixes: Vec<QualityGateFix>,
}

struct QualityGateFix {
    file: String,
    issue: String,
    model: String,
    cost_usd: f64,
}

async fn gather_quality_gates(session: &DevSession, client: &NexusClient) -> Vec<QualityGateInfo> {
    let swarm_id = match session.swarm_id.as_ref() {
        Some(id) => id,
        None => return Vec::new(),
    };

    let gates_data = match client
        .get(&format!(
            "/api/hexflo/quality-gate?swarm_id={}",
            swarm_id
        ))
        .await
    {
        Ok(data) => data,
        Err(_) => return Vec::new(),
    };

    let gates_arr = match gates_data.as_array() {
        Some(arr) => arr,
        None => return Vec::new(),
    };

    let mut result = Vec::new();
    for g in gates_arr {
        let gate_id = g["id"].as_str().unwrap_or("").to_string();
        let tier = g["tier"].as_u64().unwrap_or(0) as u32;
        let status = g["status"].as_str().unwrap_or("pending").to_string();
        let score = g["score"].as_u64().map(|s| s as u32);
        let grade = g["grade"].as_str().map(|s| s.to_string());
        let iterations = g["iterations"].as_u64().unwrap_or(1) as u32;

        // Fetch fixes for this gate
        let mut fixes = Vec::new();
        if !gate_id.is_empty() {
            if let Ok(fixes_data) = client
                .get(&format!("/api/hexflo/quality-gate/{}/fixes", gate_id))
                .await
            {
                if let Some(fixes_arr) = fixes_data.as_array() {
                    for f in fixes_arr {
                        fixes.push(QualityGateFix {
                            file: f["file"].as_str().unwrap_or("").to_string(),
                            issue: f["issue"].as_str().unwrap_or("").to_string(),
                            model: f["model"].as_str().unwrap_or("").to_string(),
                            cost_usd: f["cost_usd"].as_f64().unwrap_or(0.0),
                        });
                    }
                }
            }
        }

        result.push(QualityGateInfo {
            tier,
            status,
            score,
            grade,
            iterations,
            fixes,
        });
    }

    result.sort_by_key(|g| g.tier);
    result
}

struct GitInfo {
    created: Vec<String>,
    modified: Vec<String>,
    deleted: Vec<String>,
}

fn gather_git_info(session: &DevSession) -> Option<GitInfo> {
    // Collect all files this session touched
    let mut created = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();

    // ADR file
    if let Some(ref path) = session.adr_path {
        if Path::new(path).exists() {
            created.push(path.clone());
        }
    }

    // Workplan file
    if let Some(ref path) = session.workplan_path {
        if Path::new(path).exists() {
            created.push(path.clone());
        }
    }

    // Generated code files — scan for files matching step patterns
    // Check git status for untracked/modified files in src/ and tests/
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain", "--no-renames"])
        .output()
        .ok()?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.len() < 4 {
            continue;
        }
        let status = &line[..2];
        let file = line[3..].trim().to_string();

        // Only include files likely generated by this session
        let dominated = file.starts_with("src/core/")
            || file.starts_with("src/adapters/")
            || file.starts_with("tests/")
            || file.starts_with("docs/adrs/")
            || file.starts_with("docs/workplans/");

        if !dominated {
            continue;
        }

        match status.trim() {
            "??" | "A" | "A " => {
                if !created.contains(&file) {
                    created.push(file);
                }
            }
            "M" | " M" | "M " | "MM" => {
                modified.push(file);
            }
            "D" | " D" | "D " => {
                deleted.push(file);
            }
            _ => {}
        }
    }

    created.sort();
    modified.sort();
    deleted.sort();

    Some(GitInfo {
        created,
        modified,
        deleted,
    })
}

// ── JSON output ─────────────────────────────────────────────────────────

fn print_json_report(
    session: &DevSession,
    project_id: &Option<String>,
    adr_info: &Option<AdrInfo>,
    workplan_info: &Option<WorkplanInfo>,
    swarm_info: &Option<SwarmInfo>,
    git_info: &Option<GitInfo>,
    quality_gates: &[QualityGateInfo],
) {
    let mut report = serde_json::json!({
        "session": {
            "id": session.id,
            "feature": session.feature_description,
            "status": format!("{}", session.status),
            "started": session.created_at,
            "ended": session.updated_at,
            "phase": format!("{}", session.current_phase),
            "cost_usd": session.total_cost_usd,
            "tokens": session.total_tokens,
            "steps_completed": session.completed_steps,
            "models": session.model_selections,
            "agent_id": session.agent_id,
            "project_id": project_id,
        }
    });

    if let Some(ref qr) = session.quality_result {
        report["quality_gate"] = serde_json::json!({
            "grade": qr.grade,
            "score": qr.score,
            "iterations": qr.iterations,
            "compile_pass": qr.compile_pass,
            "compile_language": qr.compile_language,
            "test_pass": qr.test_pass,
            "tests_passed": qr.tests_passed,
            "tests_failed": qr.tests_failed,
            "violations_found": qr.violations_found,
            "violations_fixed": qr.violations_fixed,
            "fix_cost_usd": qr.fix_cost_usd,
            "fix_tokens": qr.fix_tokens,
        });
    }

    if let Some(ref adr) = adr_info {
        report["adr"] = serde_json::json!({
            "path": adr.path,
            "lines": adr.lines,
            "size_bytes": adr.size,
            "title": adr.title,
        });
    }

    if let Some(ref wp) = workplan_info {
        report["workplan"] = serde_json::json!({
            "path": wp.path,
            "title": wp.title,
            "total_steps": wp.total_steps,
            "tier_summary": wp.tier_summary,
            "steps": wp.steps.iter().map(|s| serde_json::json!({
                "id": s.id,
                "description": s.description,
                "adapter": s.adapter,
                "tier": s.tier,
            })).collect::<Vec<_>>(),
        });
    }

    if let Some(ref si) = swarm_info {
        report["swarm"] = serde_json::json!({
            "id": si.id,
            "name": si.name,
            "topology": si.topology,
            "status": si.status,
            "tasks_total": si.tasks_total,
            "tasks_completed": si.tasks_completed,
            "tasks": si.tasks.iter().map(|t| serde_json::json!({
                "id": t.id,
                "title": t.title,
                "status": t.status,
            })).collect::<Vec<_>>(),
        });
    }

    if let Some(ref gi) = git_info {
        report["files"] = serde_json::json!({
            "created": gi.created,
            "modified": gi.modified,
            "deleted": gi.deleted,
        });
    }

    if !quality_gates.is_empty() {
        report["quality_gates"] = serde_json::json!(quality_gates.iter().map(|g| {
            let mut gate = serde_json::json!({
                "tier": g.tier,
                "status": g.status,
                "score": g.score,
                "grade": g.grade,
                "iterations": g.iterations,
            });
            if !g.fixes.is_empty() {
                gate["fixes"] = serde_json::json!(g.fixes.iter().map(|f| serde_json::json!({
                    "file": f.file,
                    "issue": f.issue,
                    "model": f.model,
                    "cost_usd": f.cost_usd,
                })).collect::<Vec<_>>());
            }
            gate
        }).collect::<Vec<_>>());
    }

    // Agent reports in JSON
    let agent_calls: Vec<_> = session.tool_calls.iter()
        .filter(|c| c.phase.starts_with("agent-"))
        .collect();
    if !agent_calls.is_empty() {
        report["agent_reports"] = serde_json::json!(agent_calls.iter().map(|c| serde_json::json!({
            "role": c.phase.strip_prefix("agent-").unwrap_or(&c.phase),
            "status": c.status,
            "duration_ms": c.duration_ms,
            "cost_usd": c.cost_usd,
            "objective": c.detail,
        })).collect::<Vec<_>>());
    }

    if !session.tool_calls.is_empty() {
        report["tool_calls"] = serde_json::json!(session.tool_calls.iter().map(|c| serde_json::json!({
            "timestamp": c.timestamp,
            "phase": c.phase,
            "tool": c.tool,
            "model": c.model,
            "tokens": c.tokens,
            "cost_usd": c.cost_usd,
            "duration_ms": c.duration_ms,
            "status": c.status,
            "detail": c.detail,
        })).collect::<Vec<_>>());
    }

    println!(
        "{}",
        serde_json::to_string_pretty(&report).unwrap_or_default()
    );
}

// ── Helpers ─────────────────────────────────────────────────────────────

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max - 3])
    }
}

fn format_tokens(tokens: u64) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{:.1}K", tokens as f64 / 1_000.0)
    } else {
        format!("{}", tokens)
    }
}

fn colorize_status(status: &SessionStatus) -> String {
    match status {
        SessionStatus::Completed => "completed".green().to_string(),
        SessionStatus::InProgress => "in_progress".yellow().to_string(),
        SessionStatus::Paused => "paused".blue().to_string(),
        SessionStatus::Failed => "failed".red().to_string(),
    }
}

fn colorize_swarm_status(status: &str) -> String {
    match status {
        "active" => "active".green().to_string(),
        "completed" => "completed".dimmed().to_string(),
        _ => status.to_string(),
    }
}
