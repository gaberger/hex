//! `hex persona-prompt` — inspect + apply the STDB-backed persona prompts.
//!
//! Operator surface for the substrate landed by ADR-2026-05-23-0900.
//! Three subcommands:
//!
//! - `list` — table of every row in `persona_prompt`
//! - `show <role>` — full classify_body for one role (+ optional --reason)
//! - `apply <role> --file <path>` — read a markdown proposal artifact,
//!   extract the proposed classify/reason bodies, call the STDB
//!   `persona_prompt_apply` reducer
//!
//! v1 of the operator-as-supervisor write path: the operator vouches
//! for the proposal (typically after spawning adversarial-red +
//! adversarial-blue review by hand). A future ADR will wrap apply
//! with the automated red/blue/judge gate (Path B item 4).

use anyhow::{anyhow, Context, Result};
use clap::Subcommand;
use serde_json::{json, Value};

const DEFAULT_STDB_HOST: &str = "http://127.0.0.1:3033";
const DEFAULT_HEX_DB: &str = "hex";

#[derive(Subcommand, Debug)]
pub enum PersonaPromptAction {
    /// List every row in the persona_prompt table.
    List,

    /// Show the full classify_body (and optionally reason_body) for one role.
    Show {
        /// Role name (e.g. "cto", "cpo", "engineering-lead").
        role: String,
        /// Also print the reason_body (off by default — usually identical
        /// to classify_body in v1).
        #[arg(long)]
        reason: bool,
    },

    /// Apply a new prompt body to a role via the persona_prompt_apply
    /// reducer. Reads the proposed body from a markdown file extracted
    /// from a persona-prompt-proposal-<role>-<date>.md audit artifact.
    ///
    /// The file is treated as the raw body content — it should be the
    /// system_prompt text, NOT the whole YAML or the whole audit doc.
    /// (Use `--from-audit` to instead read the proposed YAML's
    /// system_prompt block from an audit-doc markdown.)
    Apply {
        /// Role name.
        role: String,
        /// Path to a file containing the new body text.
        #[arg(long)]
        file: String,
        /// If set, treat `--file` as a persona-prompt-proposal audit
        /// doc and extract the system_prompt: | block automatically.
        #[arg(long)]
        from_audit: bool,
        /// Override model_preferred (defaults to qwen2.5-coder:14b).
        #[arg(long)]
        model_preferred: Option<String>,
        /// Override model_upgrade_to (defaults to claude-sonnet-4-6).
        #[arg(long)]
        model_upgrade_to: Option<String>,
        /// Use distinct reason_body from a separate file. Defaults to
        /// mirroring classify_body for v1.
        #[arg(long)]
        reason_file: Option<String>,
    },

    /// Show append-only version history for a role.
    History {
        /// Role name.
        role: String,
        /// Maximum rows to print (newest first). Default: 20.
        #[arg(long, default_value_t = 20)]
        limit: u32,
    },

    /// Revert a role's active prompt to a prior history version.
    /// Forward-only: rollback creates a NEW history row (version = max+1)
    /// with the prior body's content. Use `hex persona-prompt history`
    /// to find the version number you want to revert to. Omit `--to`
    /// to revert to the most-recent superseded version (one-shot undo).
    Rollback {
        /// Role name.
        role: String,
        /// Version to revert to. Default 0 = most recent superseded.
        #[arg(long, default_value_t = 0)]
        to: u64,
    },

    /// Run the full hive-improve loop for a role: GROUND failure
    /// evidence → DISPATCH a rewriter → DEBATE via adversarial-red
    /// (Anthropic) + adversarial-blue (Ollama) → JUDGE arbitration →
    /// APPLY if approved. Operator-triggered version of the autonomous
    /// improver (ADR-2026-05-23-0900 Path B item 5).
    ///
    /// Provider divergence is enforced by the agent YAMLs' provider_lock
    /// fields (ADR-2026-05-23-0900 Path B item 1, closed today). Red
    /// runs on claude-sonnet-4-6 (Anthropic), blue on devstral-small-2:24b
    /// (local Ollama) — if the verdicts agree, it's not from a shared
    /// blind-spot.
    ///
    /// The auto-rollback observer (item 7) catches regressions after
    /// apply, so this command is safe to run autonomously — even a bad
    /// improvement reverts itself within ~60s past the grace period.
    Improve {
        /// Role name.
        role: String,
        /// Show the proposal + verdicts but do NOT apply.
        #[arg(long)]
        dry_run: bool,
        /// Override the rewriter model (default: qwen2.5-coder:14b).
        #[arg(long)]
        rewriter_model: Option<String>,
        /// Skip the red/blue debate (operator-trusted apply — only use
        /// for testing or when divergence isn't required).
        #[arg(long)]
        skip_debate: bool,
        /// Where to write the audit-trail markdown. Defaults to
        /// docs/specs/persona-prompt-improve-<role>-<UTC>.md.
        #[arg(long)]
        audit_out: Option<String>,
    },
}

pub async fn run(action: PersonaPromptAction) -> Result<()> {
    match action {
        PersonaPromptAction::List => list().await,
        PersonaPromptAction::Show { role, reason } => show(&role, reason).await,
        PersonaPromptAction::Apply {
            role,
            file,
            from_audit,
            model_preferred,
            model_upgrade_to,
            reason_file,
        } => {
            apply(
                &role,
                &file,
                from_audit,
                model_preferred.as_deref(),
                model_upgrade_to.as_deref(),
                reason_file.as_deref(),
            )
            .await
        }
        PersonaPromptAction::History { role, limit } => history(&role, limit).await,
        PersonaPromptAction::Rollback { role, to } => rollback(&role, to).await,
        PersonaPromptAction::Improve {
            role,
            dry_run,
            rewriter_model,
            skip_debate,
            audit_out,
        } => {
            improve(
                &role,
                dry_run,
                rewriter_model.as_deref(),
                skip_debate,
                audit_out.as_deref(),
            )
            .await
        }
    }
}

fn stdb_host() -> String {
    std::env::var("HEX_STDB_HOST").unwrap_or_else(|_| DEFAULT_STDB_HOST.to_string())
}
fn hex_db() -> String {
    std::env::var("HEX_STDB_HEXFLO_DB").unwrap_or_else(|_| DEFAULT_HEX_DB.to_string())
}

/// Provider divergence class for a model id. Mirrors the helper in
/// hex-nexus/src/orchestration/hive_improver.rs — kept duplicated
/// here because the CLI and nexus crates have no shared module yet
/// for prompt-pipeline helpers, and pulling either into hex-core
/// would drag inference details into the domain layer.
fn provider_class_of(model: &str) -> &'static str {
    let m = model.to_ascii_lowercase();
    if m.starts_with("claude-") || m.starts_with("anthropic/") {
        "anthropic"
    } else if m.starts_with("gpt-") || m.starts_with("openai/") || m.starts_with("o1-") {
        "openai"
    } else if m.starts_with("openrouter/") {
        "openrouter"
    } else if m.contains(':') {
        "ollama"
    } else {
        "unknown"
    }
}

async fn sql(query: &str) -> Result<Vec<Vec<Value>>> {
    let url = format!("{}/v1/database/{}/sql", stdb_host(), hex_db());
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;
    let res = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(query.to_string())
        .send()
        .await
        .with_context(|| format!("POST {}", url))?;
    if !res.status().is_success() {
        return Err(anyhow!(
            "STDB SQL HTTP {} — {}",
            res.status(),
            res.text().await.unwrap_or_default()
        ));
    }
    let body: Value = res.json().await.context("parse STDB JSON")?;
    // STDB SQL response: top-level array of result-sets; each has `rows`
    // as a list of column tuples.
    let mut out: Vec<Vec<Value>> = Vec::new();
    let collect = |rows: &Value, out: &mut Vec<Vec<Value>>| {
        if let Some(arr) = rows.as_array() {
            for r in arr {
                if let Some(cols) = r.as_array() {
                    out.push(cols.clone());
                }
            }
        }
    };
    if let Some(arr) = body.as_array() {
        for rs in arr {
            if let Some(rows) = rs.get("rows") {
                collect(rows, &mut out);
            }
        }
    } else if let Some(rows) = body.get("rows") {
        collect(rows, &mut out);
    }
    Ok(out)
}

async fn list() -> Result<()> {
    let rows = sql(
        "SELECT role, model_preferred, model_upgrade_to, seeded_by FROM persona_prompt",
    )
    .await?;
    if rows.is_empty() {
        println!("⬡ persona_prompt is empty (no rows).");
        println!("   Cold-start seeding fires on next nexus tick — `hex nexus stop && hex nexus start`.");
        return Ok(());
    }
    println!(
        "{:18}  {:22}  {:22}  {}",
        "role", "model_preferred", "model_upgrade_to", "applied_by"
    );
    println!(
        "{:18}  {:22}  {:22}  {}",
        "------------------",
        "----------------------",
        "----------------------",
        "------------------------------"
    );
    for r in rows {
        let role = r.first().and_then(|v| v.as_str()).unwrap_or("?");
        let pref = r.get(1).and_then(|v| v.as_str()).unwrap_or("?");
        let upgr = r.get(2).and_then(|v| v.as_str()).unwrap_or("?");
        let by = r.get(3).and_then(|v| v.as_str()).unwrap_or("?");
        // Truncate the principal hash for readability — full hash still
        // queryable via `hex persona-prompt show <role>`.
        let by_short = if by.starts_with("applied:") {
            format!("applied:{}…", by.chars().skip(8).take(12).collect::<String>())
        } else {
            format!("{}…", by.chars().take(20).collect::<String>())
        };
        println!("{:18}  {:22}  {:22}  {}", role, pref, upgr, by_short);
    }
    Ok(())
}

async fn show(role: &str, reason: bool) -> Result<()> {
    let safe = role.replace('\'', "''");
    let rows = sql(&format!(
        "SELECT role, classify_body, reason_body, model_preferred, model_upgrade_to, seeded_by \
         FROM persona_prompt WHERE role = '{}'",
        safe
    ))
    .await?;
    let row = rows
        .first()
        .ok_or_else(|| anyhow!("no persona_prompt row for role '{}'", role))?;
    let role_str = row.first().and_then(|v| v.as_str()).unwrap_or("?");
    let classify = row.get(1).and_then(|v| v.as_str()).unwrap_or("");
    let reason_body = row.get(2).and_then(|v| v.as_str()).unwrap_or("");
    let pref = row.get(3).and_then(|v| v.as_str()).unwrap_or("?");
    let upgr = row.get(4).and_then(|v| v.as_str()).unwrap_or("?");
    let by = row.get(5).and_then(|v| v.as_str()).unwrap_or("?");

    println!("=== persona_prompt: {} ===", role_str);
    println!("  model_preferred:  {}", pref);
    println!("  model_upgrade_to: {}", upgr);
    println!("  applied_by:       {}", by);
    println!(
        "  classify_body:    {} bytes",
        classify.len()
    );
    println!(
        "  reason_body:      {} bytes",
        reason_body.len()
    );
    println!();
    println!("--- classify_body ---");
    println!("{}", classify);
    if reason {
        println!();
        println!("--- reason_body ---");
        println!("{}", reason_body);
    }
    Ok(())
}

/// Parse the `system_prompt: |` block out of a YAML audit doc. Used by
/// `--from-audit`. Returns the de-indented body content.
fn extract_system_prompt_from_audit(audit_md: &str) -> Result<String> {
    // Find the proposed-yaml fence block first.
    let mut in_yaml = false;
    let mut yaml_lines: Vec<&str> = Vec::new();
    for line in audit_md.lines() {
        if line.starts_with("```yaml") {
            in_yaml = true;
            continue;
        }
        if in_yaml && line.starts_with("```") {
            break;
        }
        if in_yaml {
            yaml_lines.push(line);
        }
    }
    if yaml_lines.is_empty() {
        return Err(anyhow!(
            "no fenced ```yaml block found in audit doc — expected \
             a proposed cto.yml (or similar) embedded in the spec"
        ));
    }
    // Walk YAML looking for `system_prompt: |` block scalar; collect
    // indented lines until the indent drops.
    let mut out: Vec<String> = Vec::new();
    let mut collecting = false;
    let mut base_indent: Option<usize> = None;
    for line in &yaml_lines {
        if !collecting {
            if line.trim_start().starts_with("system_prompt:") && line.contains("|") {
                collecting = true;
            }
            continue;
        }
        // Indented line OR blank line → still in block. Dedented non-blank → block ends.
        let trimmed_len = line.trim_start().len();
        let indent = line.len() - trimmed_len;
        if trimmed_len == 0 {
            out.push(String::new());
            continue;
        }
        if base_indent.is_none() {
            base_indent = Some(indent);
        }
        let bi = base_indent.unwrap();
        if indent < bi {
            break;
        }
        out.push(line[bi.min(line.len())..].to_string());
    }
    let body = out.join("\n");
    // Strip trailing blank lines.
    Ok(body.trim_end().to_string())
}

async fn apply(
    role: &str,
    file: &str,
    from_audit: bool,
    model_preferred: Option<&str>,
    model_upgrade_to: Option<&str>,
    reason_file: Option<&str>,
) -> Result<()> {
    let raw = tokio::fs::read_to_string(file)
        .await
        .with_context(|| format!("read {}", file))?;
    let classify_body = if from_audit {
        extract_system_prompt_from_audit(&raw)?
    } else {
        raw.trim_end().to_string()
    };
    if classify_body.is_empty() {
        return Err(anyhow!(
            "extracted classify_body is empty — check the file path and (if --from-audit) \
             that the audit doc has a ```yaml block with a `system_prompt: |` scalar"
        ));
    }
    if classify_body.len() > 8192 {
        return Err(anyhow!(
            "classify_body is {} bytes; STDB cap is 8192. \
             Trim the proposal body or split into per-intent variants (deferred to a future ADR).",
            classify_body.len()
        ));
    }
    let reason_body = match reason_file {
        Some(p) => tokio::fs::read_to_string(p)
            .await
            .with_context(|| format!("read --reason-file {}", p))?
            .trim_end()
            .to_string(),
        None => classify_body.clone(),
    };
    if reason_body.len() > 8192 {
        return Err(anyhow!(
            "reason_body is {} bytes; STDB cap is 8192.",
            reason_body.len()
        ));
    }

    let model_pref = model_preferred.unwrap_or("qwen2.5-coder:14b").to_string();
    let model_upg = model_upgrade_to.unwrap_or("claude-sonnet-4-6").to_string();

    // Call the reducer.
    let url = format!(
        "{}/v1/database/{}/call/persona_prompt_apply",
        stdb_host(),
        hex_db()
    );
    let payload = json!([role, classify_body, reason_body, model_pref, model_upg]);
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let res = http
        .post(&url)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("POST {}", url))?;
    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        return Err(anyhow!(
            "persona_prompt_apply rejected — HTTP {}: {}",
            status,
            body
        ));
    }

    println!("⬡ persona_prompt_apply OK");
    println!("  role:             {}", role);
    println!("  classify_body:    {} bytes", classify_body.len());
    println!("  reason_body:      {} bytes", reason_body.len());
    println!("  model_preferred:  {}", model_pref);
    println!("  model_upgrade_to: {}", model_upg);
    println!();
    println!("  Cache refresh fires on next supervisor tick (≤5s).");
    println!("  Inspect via: hex persona-prompt show {}", role);
    Ok(())
}

async fn history(role: &str, limit: u32) -> Result<()> {
    let safe = role.replace('\'', "''");
    let rows = sql(&format!(
        "SELECT role, version, event_kind, applied_at, applied_by, \
                superseded_at, superseded_by_version \
         FROM persona_prompt_history WHERE role = '{}'",
        safe
    ))
    .await?;
    if rows.is_empty() {
        println!("⬡ no history for role '{}'.", role);
        return Ok(());
    }
    // Newest first.
    let mut rows = rows;
    rows.sort_by(|a, b| {
        let av = a.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
        let bv = b.get(1).and_then(|v| v.as_u64()).unwrap_or(0);
        bv.cmp(&av)
    });
    println!(
        "{:>3}  {:11}  {:36}  {:14}  {}",
        "v", "event", "applied_at", "active?", "applied_by"
    );
    println!(
        "{:>3}  {:11}  {:36}  {:14}  {}",
        "---", "-----------", "------------------------------------", "--------------", "----------"
    );
    for r in rows.into_iter().take(limit as usize) {
        let v = r.get(1).and_then(|x| x.as_u64()).unwrap_or(0);
        let evt = r.get(2).and_then(|x| x.as_str()).unwrap_or("?");
        let at = format_timestamp_micros(r.get(3));
        // STDB serializes Option<T> as a `[variant_tag, payload]` 2-tuple:
        //   - [0, [value]]  → Some(value)
        //   - [1, []]       → None
        // The history row's `superseded_at` is None on the currently-active
        // version, Some(timestamp) on retired versions. Tag the active one
        // explicitly so the operator can see at a glance which row drives
        // the live persona_prompt cache.
        let active = r
            .get(5)
            .and_then(|v| v.as_array())
            .and_then(|a| a.first())
            .and_then(|t| t.as_u64())
            .map(|tag| tag == 1)
            .unwrap_or(true);
        let active_str = if active {
            "ACTIVE".to_string()
        } else {
            // Option<u64> serializes as `[0, N]` — primitive payload sits
            // directly at index 1, NOT wrapped in another array. (Compare
            // Option<Timestamp> which is `[0, [N]]` because Timestamp is
            // a Product type.) The mismatch is real — STDB's BSATN treats
            // primitives and product types differently in this position.
            let by_v = r
                .get(6)
                .and_then(|v| v.as_array())
                .and_then(|a| a.get(1))
                .and_then(|n| n.as_u64())
                .unwrap_or(0);
            format!("superseded→v{}", by_v)
        };
        let by = r.get(4).and_then(|x| x.as_str()).unwrap_or("?");
        let by_short = if by.starts_with("applied:") {
            format!("applied:{}…", by.chars().skip(8).take(12).collect::<String>())
        } else if by.starts_with("rollback:") {
            format!("rollback:{}…", by.chars().skip(9).take(12).collect::<String>())
        } else {
            format!("{}…", by.chars().take(20).collect::<String>())
        };
        println!("{:>3}  {:11}  {:36}  {:14}  {}", v, evt, at, active_str, by_short);
    }
    Ok(())
}

/// Best-effort parse of an STDB Timestamp cell into RFC-3339. STDB SQL
/// surfaces Timestamps in several encodings depending on column type:
///   - Required `Timestamp` columns: tagged array `[micros_i64]` OR string
///   - `Option<Timestamp>` columns: `[variant_tag, [...]]`, where tag=0
///     means Some(timestamp) with payload `[micros_i64]`, tag=1 = None
///   - Pretty-printed via Debug as `Timestamp { __...__: N }`
/// This helper handles all three by walking the value defensively.
fn format_timestamp_micros(cell: Option<&Value>) -> String {
    let v = match cell {
        Some(v) => v,
        None => return "?".to_string(),
    };

    // Direct array shape: [micros] (required field) or [tag, payload] (Option).
    if let Some(arr) = v.as_array() {
        // Required Timestamp column: STDB returns [micros_i64] as the cell.
        if arr.len() == 1 {
            if let Some(n) = arr.first().and_then(|x| x.as_i64()) {
                return micros_to_rfc3339(n);
            }
        }
        // Option<Timestamp>: [tag, payload]. tag=0 is Some, tag=1 is None.
        if arr.len() == 2 {
            let tag = arr.first().and_then(|x| x.as_u64()).unwrap_or(1);
            if tag == 1 {
                return "—".to_string();
            }
            if let Some(payload) = arr.get(1).and_then(|x| x.as_array()) {
                if let Some(n) = payload.first().and_then(|x| x.as_i64()) {
                    return micros_to_rfc3339(n);
                }
            }
        }
    }

    let s = match v.as_str() {
        Some(s) => s.to_string(),
        None => v.to_string(),
    };
    // Already ISO-8601?
    if s.contains('T') && s.contains(':') {
        return s.chars().take(36).collect();
    }
    // Debug-formatted Timestamp.
    let key = "__timestamp_micros_since_unix_epoch__";
    if let Some(i) = s.find(key) {
        let digits: String = s[i + key.len()..]
            .chars()
            .skip_while(|c| !c.is_ascii_digit() && *c != '-')
            .take_while(|c| c.is_ascii_digit() || *c == '-')
            .collect();
        if let Ok(n) = digits.parse::<i64>() {
            return micros_to_rfc3339(n);
        }
    }
    "?".to_string()
}

fn micros_to_rfc3339(micros: i64) -> String {
    let secs = micros / 1_000_000;
    let nsec = ((micros % 1_000_000) * 1_000) as u32;
    chrono::DateTime::<chrono::Utc>::from_timestamp(secs, nsec)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_else(|| "?".to_string())
}

async fn rollback(role: &str, to: u64) -> Result<()> {
    let url = format!(
        "{}/v1/database/{}/call/persona_prompt_rollback",
        stdb_host(),
        hex_db()
    );
    let payload = json!([role, to]);
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let res = http
        .post(&url)
        .json(&payload)
        .send()
        .await
        .with_context(|| format!("POST {}", url))?;
    if !res.status().is_success() {
        return Err(anyhow!(
            "persona_prompt_rollback rejected — HTTP {}: {}",
            res.status(),
            res.text().await.unwrap_or_default()
        ));
    }
    if to == 0 {
        println!("⬡ persona_prompt_rollback OK — reverted '{}' to most recent prior version", role);
    } else {
        println!("⬡ persona_prompt_rollback OK — reverted '{}' to v{}", role, to);
    }
    println!("  Cache refresh fires on next supervisor tick (≤5s).");
    println!("  Inspect via: hex persona-prompt show {}", role);
    println!("              hex persona-prompt history {}", role);
    Ok(())
}

// ─────────────────────────────────────────────────────────────────
// HIVE-IMPROVE orchestrator (ADR-2026-05-23-0900 Path B item 5)
// ─────────────────────────────────────────────────────────────────

/// Result of a single inference phase.
#[derive(Clone, Debug)]
struct PhaseOutput {
    role_label: String,
    model: String,
    content: String,
    input_tokens: u32,
    output_tokens: u32,
    duration_ms: u128,
}

/// Verdict extracted from an adversarial reviewer's reply. Parsed
/// case-insensitively from the first occurrence of "verdict:" in the
/// reply text. Defaults to Reject on parse failure (fail-closed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Approve,
    ApproveWithChanges,
    Reject,
}

impl Verdict {
    fn from_text(text: &str) -> Verdict {
        let lower = text.to_lowercase();
        // Prefer the most explicit phrasing first.
        let needle_idx = lower.find("verdict:").map(|i| i + "verdict:".len());
        let tail = needle_idx.map(|i| &lower[i..]).unwrap_or(&lower);
        let head: String = tail.chars().take(80).collect();
        if head.contains("approve-with-changes") || head.contains("approve with changes") {
            return Verdict::ApproveWithChanges;
        }
        if head.contains("reject") {
            return Verdict::Reject;
        }
        if head.contains("approve") {
            return Verdict::Approve;
        }
        Verdict::Reject
    }

    fn is_approving(self) -> bool {
        matches!(self, Verdict::Approve | Verdict::ApproveWithChanges)
    }

    fn as_str(self) -> &'static str {
        match self {
            Verdict::Approve => "approve",
            Verdict::ApproveWithChanges => "approve-with-changes",
            Verdict::Reject => "reject",
        }
    }
}

/// Call the local nexus inference endpoint. Same shape as org_responder
/// uses internally. Returns the response content + token counts.
async fn complete_inference(
    http: &reqwest::Client,
    model: &str,
    system: Option<&str>,
    user: &str,
    max_tokens: u32,
) -> Result<PhaseOutput> {
    let url = "http://127.0.0.1:5555/api/inference/complete";
    let mut messages = Vec::new();
    if let Some(sys) = system {
        messages.push(serde_json::json!({"role": "system", "content": sys}));
    }
    messages.push(serde_json::json!({"role": "user", "content": user}));
    let body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
    });
    let start = std::time::Instant::now();
    let res = http
        .post(url)
        .json(&body)
        .send()
        .await
        .with_context(|| format!("POST {} (model={})", url, model))?;
    let duration_ms = start.elapsed().as_millis();
    if !res.status().is_success() {
        return Err(anyhow!(
            "{} returned HTTP {}: {}",
            model,
            res.status(),
            res.text().await.unwrap_or_default()
        ));
    }
    let resp: Value = res
        .json()
        .await
        .with_context(|| format!("parse JSON from {}", model))?;
    Ok(PhaseOutput {
        role_label: String::new(),
        model: resp
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or(model)
            .to_string(),
        content: resp
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        input_tokens: resp.get("input_tokens").and_then(|v| v.as_u64()).unwrap_or(0) as u32,
        output_tokens: resp
            .get("output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32,
        duration_ms,
    })
}

/// Pull failure evidence for the GROUND phase. Returns a markdown-formatted
/// chunk to inject into the rewriter prompt. Empty (or near-empty) when the
/// persona has had no failures — that's a valid signal too ("rewrite is
/// speculative, not regression-driven").
async fn ground_evidence(role: &str) -> Result<String> {
    let mut out = String::new();
    let safe = role.replace('\'', "''");

    // persona_health.recent_failures + last_failure_at
    let rows = sql(&format!(
        "SELECT recent_failures, last_failure_at, last_failure_model, last_failure_status \
         FROM persona_health WHERE role = '{}'",
        safe
    ))
    .await
    .unwrap_or_default();
    if let Some(row) = rows.first() {
        let recent = row.first().and_then(|v| v.as_u64()).unwrap_or(0);
        let last_at = row.get(1).and_then(|v| v.as_str()).unwrap_or("");
        let last_model = row.get(2).and_then(|v| v.as_str()).unwrap_or("");
        let last_status = row.get(3).and_then(|v| v.as_u64()).unwrap_or(0);
        out.push_str(&format!(
            "### persona_health\n- recent_failures (rolling 60s window): **{}**\n- last_failure_at: {}\n- last_failure_model: {}\n- last_failure_status: {}\n\n",
            recent, last_at, last_model, last_status
        ));
    } else {
        out.push_str("### persona_health\n- no row (no failures since last apply)\n\n");
    }

    // recent agent_messages — try chat-relay db; quietly skip if not queryable
    let chat_db =
        std::env::var("HEX_STDB_CHAT_DB").unwrap_or_else(|_| "chat-relay".to_string());
    let url = format!("{}/v1/database/{}/sql", stdb_host(), chat_db);
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(3))
        .build()?;
    if let Ok(res) = http
        .post(&url)
        .header("Content-Type", "text/plain")
        .body(format!(
            "SELECT id, content FROM agent_messages WHERE from_agent = '{}' LIMIT 8",
            safe
        ))
        .send()
        .await
    {
        if res.status().is_success() {
            if let Ok(body) = res.json::<Value>().await {
                let mut count = 0;
                if let Some(arr) = body.as_array() {
                    for rs in arr {
                        if let Some(rows) = rs.get("rows").and_then(|r| r.as_array()) {
                            for r in rows {
                                if let Some(cols) = r.as_array() {
                                    let content = cols.get(1).and_then(|v| v.as_str()).unwrap_or("");
                                    if !content.is_empty() {
                                        if count == 0 {
                                            out.push_str(&format!(
                                                "### recent agent_messages (from={})\n",
                                                role
                                            ));
                                        }
                                        let snippet: String =
                                            content.chars().take(220).collect();
                                        out.push_str(&format!("- {}\n", snippet));
                                        count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                if count > 0 {
                    out.push('\n');
                }
            }
        }
    }

    Ok(out)
}

async fn improve(
    role: &str,
    dry_run: bool,
    rewriter_model: Option<&str>,
    skip_debate: bool,
    audit_out: Option<&str>,
) -> Result<()> {
    println!("⬡ hive-improve: {}", role);
    println!("  GROUND → DISPATCH → DEBATE → JUDGE → APPLY");
    if dry_run {
        println!("  --dry-run: will NOT apply, audit only");
    }
    println!();

    // ── PHASE 1: GROUND
    print!("[1/5] GROUND   ... ");
    let safe = role.replace('\'', "''");
    let cur = sql(&format!(
        "SELECT classify_body, model_preferred, model_upgrade_to, seeded_by \
         FROM persona_prompt WHERE role = '{}'",
        safe
    ))
    .await?;
    let cur_row = cur
        .first()
        .ok_or_else(|| anyhow!("no persona_prompt row for role '{}' — seed it first", role))?;
    let current_body = cur_row.first().and_then(|v| v.as_str()).unwrap_or("").to_string();
    let model_preferred = cur_row
        .get(1)
        .and_then(|v| v.as_str())
        .unwrap_or("qwen2.5-coder:14b")
        .to_string();
    let model_upgrade_to = cur_row
        .get(2)
        .and_then(|v| v.as_str())
        .unwrap_or("claude-sonnet-4-6")
        .to_string();
    let seeded_by = cur_row.get(3).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let evidence = ground_evidence(role).await.unwrap_or_default();
    println!(
        "ok  (current body: {} bytes, seeded_by: {}, evidence: {} bytes)",
        current_body.len(),
        if seeded_by.len() > 28 { format!("{}…", &seeded_by[..28]) } else { seeded_by.clone() },
        evidence.len()
    );

    // ── PHASE 2: DISPATCH (rewriter)
    print!("[2/5] DISPATCH ... ");
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(600))
        .build()?;
    let rewriter = rewriter_model.unwrap_or("qwen2.5-coder:14b");
    let rewriter_system =
        "You are a prompt-rewriter for hex AIOS personas. You produce ONLY the new \
         system_prompt body text — no preamble, no fences, no commentary. The body \
         must follow the strict-JSON ClassifierResponse contract that personas in \
         hex must emit. Cap output at 4000 characters.";
    let rewriter_user = format!(
        "Persona: {}\n\nCurrent persona_prompt body:\n```\n{}\n```\n\nFailure evidence:\n{}\n\nProduce a new body that addresses the failure evidence (if any) and tightens contract adherence. Output the new body text directly — no markdown, no commentary, no fences.",
        role, current_body, evidence
    );
    let proposal = complete_inference(&http, rewriter, Some(rewriter_system), &rewriter_user, 4000)
        .await
        .context("rewriter inference failed")?;
    let proposed_body = proposal.content.trim().to_string();
    if proposed_body.is_empty() {
        anyhow::bail!("rewriter returned empty proposal — aborting");
    }
    println!(
        "ok  (model={} {}ms, {}→{} tokens, proposal: {} bytes)",
        proposal.model,
        proposal.duration_ms,
        proposal.input_tokens,
        proposal.output_tokens,
        proposed_body.len()
    );

    // ── PHASE 3: DEBATE (adversarial-red + adversarial-blue, parallel, divergent providers)
    let (red_verdict, blue_verdict, red_out, blue_out) = if skip_debate {
        print!("[3/5] DEBATE   ... skipped (--skip-debate)\n");
        (Verdict::Approve, Verdict::Approve, None, None)
    } else {
        print!("[3/5] DEBATE   ... ");
        let red_system = "You are adversarial-red — security/boundary/autonomy-escape skeptic. \
                          Hunt: prompt-injection, boundary escape via tool_plan, identity \
                          spoofing in route, operator-quorum bypass. Output starts with \
                          'Verdict: approve|approve-with-changes|reject' on the first line, \
                          followed by numbered findings (each [P0/P1/P2]). Be ruthless: \
                          if the proposal has ANY P0 hole, REJECT.";
        let blue_system = "You are adversarial-blue — correctness/spec-drift/lying-error skeptic. \
                           Hunt: places where the proposed body claims behavior the runtime \
                           won't deliver, schema field-name mismatches, untriggerable rules, \
                           contradictory directives. Output starts with 'Verdict: approve|\
                           approve-with-changes|reject' on the first line. Be ruthless on \
                           correctness gaps.";
        let user_for_review = format!(
            "Proposed new persona_prompt body for role '{}':\n\n```\n{}\n```\n\nReview it.",
            role, proposed_body
        );
        // Provider divergence: red MUST be claude-sonnet-4-6 (Anthropic),
        // blue MUST be devstral-small-2:24b (Ollama local). Provider_lock
        // enforcement in agent/mod.rs:2544 catches misconfigured YAMLs;
        // here we just pick the right model per role directly.
        let (red_res, blue_res) = tokio::join!(
            complete_inference(&http, "claude-sonnet-4-6", Some(red_system), &user_for_review, 1500),
            complete_inference(&http, "devstral-small-2:24b", Some(blue_system), &user_for_review, 1500),
        );
        let red = red_res.context("adversarial-red inference failed")?;
        let blue = blue_res.context("adversarial-blue inference failed")?;
        let red_v = Verdict::from_text(&red.content);
        let blue_v = Verdict::from_text(&blue.content);
        println!(
            "red={} blue={} (red: {}ms, blue: {}ms)",
            red_v.as_str(),
            blue_v.as_str(),
            red.duration_ms,
            blue.duration_ms
        );
        (red_v, blue_v, Some(red), Some(blue))
    };

    // ── PHASE 4: JUDGE (validation-judge arbitration)
    print!("[4/5] JUDGE    ... ");
    let judge_v = if skip_debate {
        println!("skipped (--skip-debate, judge bypass)");
        Verdict::Approve
    } else {
        let judge_system = "You are validation-judge — final arbiter. You see two \
                            adversarial verdicts (red on Anthropic, blue on Ollama — \
                            provider divergent). Output starts with 'Verdict: approve|\
                            approve-with-changes|reject' on the first line, followed by \
                            a 2-3 sentence rationale. Approve only if BOTH adversaries \
                            converge on approve OR approve-with-changes.";
        let red_text = red_out.as_ref().map(|p| p.content.as_str()).unwrap_or("");
        let blue_text = blue_out.as_ref().map(|p| p.content.as_str()).unwrap_or("");
        let user_for_judge = format!(
            "## adversarial-red (Anthropic):\n{}\n\n## adversarial-blue (Ollama):\n{}\n\n## Proposed body:\n```\n{}\n```\n\nArbitrate.",
            red_text, blue_text, proposed_body
        );
        let judge_out = complete_inference(
            &http,
            "claude-sonnet-4-6",
            Some(judge_system),
            &user_for_judge,
            800,
        )
        .await
        .context("validation-judge inference failed")?;
        let v = Verdict::from_text(&judge_out.content);
        println!("judge={} ({}ms)", v.as_str(), judge_out.duration_ms);
        v
    };

    // ── PHASE 5: APPLY (or NOT)
    print!("[5/5] APPLY    ... ");
    let approved = (skip_debate
        || (red_verdict.is_approving() && blue_verdict.is_approving()))
        && judge_v.is_approving();
    let applied = if !approved {
        println!("NOT applied (verdict chain rejected)");
        false
    } else if dry_run {
        println!("--dry-run; would apply ({} bytes)", proposed_body.len());
        false
    } else if skip_debate {
        // skip_debate is an operator escape hatch — no red/blue verdicts
        // to pass to the gated reducer, so call the non-gated path. The
        // operator's judgment is the sole authority in this branch.
        let url = format!(
            "{}/v1/database/{}/call/persona_prompt_apply",
            stdb_host(),
            hex_db()
        );
        let payload = serde_json::json!([
            role,
            proposed_body,
            proposed_body,
            model_preferred,
            model_upgrade_to,
        ]);
        let res = http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("apply call failed")?;
        if res.status().is_success() {
            println!("applied → persona_prompt updated (non-gated, skip-debate)");
            true
        } else {
            println!(
                "apply REJECTED — HTTP {}: {}",
                res.status(),
                res.text().await.unwrap_or_default()
            );
            false
        }
    } else {
        // Default path: gated apply. The reducer re-checks verdicts +
        // provider divergence even though we already passed them in-
        // process — STDB is the contract surface, not the CLI.
        let red_model = red_out
            .as_ref()
            .map(|p| p.model.as_str())
            .unwrap_or("claude-sonnet-4-6");
        let blue_model = blue_out
            .as_ref()
            .map(|p| p.model.as_str())
            .unwrap_or("devstral-small-2:24b");
        let red_provider = provider_class_of(red_model);
        let blue_provider = provider_class_of(blue_model);
        let url = format!(
            "{}/v1/database/{}/call/persona_prompt_apply_gated",
            stdb_host(),
            hex_db()
        );
        let payload = serde_json::json!([
            role,
            proposed_body,
            proposed_body,
            model_preferred,
            model_upgrade_to,
            red_provider,
            red_verdict.as_str(),
            blue_provider,
            blue_verdict.as_str(),
            judge_v.as_str(),
        ]);
        let res = http
            .post(&url)
            .json(&payload)
            .send()
            .await
            .context("gated apply call failed")?;
        if res.status().is_success() {
            println!("applied → persona_prompt updated (gated)");
            true
        } else {
            println!(
                "gated apply REJECTED — HTTP {}: {}",
                res.status(),
                res.text().await.unwrap_or_default()
            );
            false
        }
    };

    // ── Audit doc
    let audit_path = audit_out.map(String::from).unwrap_or_else(|| {
        let stamp = chrono::Utc::now().format("%Y-%m-%dT%H%M%S");
        format!("docs/specs/persona-prompt-improve-{}-{}.md", role, stamp)
    });
    let audit = build_audit_doc(
        role,
        &current_body,
        &proposed_body,
        &evidence,
        &proposal,
        red_out.as_ref(),
        blue_out.as_ref(),
        red_verdict,
        blue_verdict,
        judge_v,
        applied,
        dry_run,
        skip_debate,
    );
    if let Some(parent) = std::path::Path::new(&audit_path).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    tokio::fs::write(&audit_path, &audit)
        .await
        .with_context(|| format!("write audit doc {}", audit_path))?;
    println!();
    println!("  audit: {} ({} bytes)", audit_path, audit.len());
    if applied {
        println!("  ⬡ check: hex persona-prompt show {}", role);
        println!("  ⬡ history: hex persona-prompt history {}", role);
        println!("  ⬡ auto-rollback observer will catch regressions within ~60s past grace");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_audit_doc(
    role: &str,
    current_body: &str,
    proposed_body: &str,
    evidence: &str,
    proposal: &PhaseOutput,
    red: Option<&PhaseOutput>,
    blue: Option<&PhaseOutput>,
    red_v: Verdict,
    blue_v: Verdict,
    judge_v: Verdict,
    applied: bool,
    dry_run: bool,
    skip_debate: bool,
) -> String {
    let mut out = String::new();
    let stamp = chrono::Utc::now().to_rfc3339();
    out.push_str(&format!(
        "# hive-improve: {} — {}\n\n",
        role, stamp
    ));
    out.push_str(&format!(
        "**Status:** {}\n",
        if applied {
            "APPLIED"
        } else if dry_run {
            "DRY-RUN (proposal only)"
        } else {
            "REJECTED (verdict chain)"
        }
    ));
    out.push_str(&format!(
        "**Verdict chain:** red={} → blue={} → judge={}  {}\n\n",
        red_v.as_str(),
        blue_v.as_str(),
        judge_v.as_str(),
        if skip_debate { "(debate skipped)" } else { "" }
    ));
    out.push_str("## Phase 1 — GROUND\n\n");
    out.push_str(&format!("Current body: {} bytes\n\n", current_body.len()));
    out.push_str(evidence);
    if evidence.is_empty() {
        out.push_str("(no failure evidence — rewrite is speculative)\n\n");
    }
    out.push_str("## Phase 2 — DISPATCH (rewriter)\n\n");
    out.push_str(&format!(
        "Model: {}\n{}ms, {}→{} tokens\n\n### Proposed body ({} bytes)\n```\n{}\n```\n\n",
        proposal.model, proposal.duration_ms, proposal.input_tokens, proposal.output_tokens,
        proposed_body.len(), proposed_body
    ));
    if let Some(r) = red {
        out.push_str(&format!(
            "## Phase 3a — adversarial-red ({})\n\n**Verdict:** {}\n\n```\n{}\n```\n\n",
            r.model, red_v.as_str(), r.content
        ));
    }
    if let Some(b) = blue {
        out.push_str(&format!(
            "## Phase 3b — adversarial-blue ({})\n\n**Verdict:** {}\n\n```\n{}\n```\n\n",
            b.model, blue_v.as_str(), b.content
        ));
    }
    out.push_str(&format!(
        "## Phase 4 — JUDGE\n\n**Verdict:** {}\n\n",
        judge_v.as_str()
    ));
    out.push_str(&format!(
        "## Phase 5 — APPLY\n\n{}\n",
        if applied {
            "Applied via `persona_prompt_apply_gated` (Path B item 4 — verdict + provider divergence checked at the reducer). Auto-rollback observer will revert if regressions appear."
        } else if dry_run {
            "Skipped (--dry-run)."
        } else {
            "Not applied — verdict chain did not converge on approve."
        }
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verdict_parses_explicit_verbiage() {
        assert_eq!(Verdict::from_text("Verdict: approve\nbody"), Verdict::Approve);
        assert_eq!(Verdict::from_text("Verdict: approve-with-changes"), Verdict::ApproveWithChanges);
        assert_eq!(Verdict::from_text("Verdict: reject\n# Findings"), Verdict::Reject);
        // Case-insensitive
        assert_eq!(Verdict::from_text("VERDICT: APPROVE"), Verdict::Approve);
    }

    #[test]
    fn verdict_fails_closed_on_no_verdict_line() {
        // No verdict keyword → Reject (fail-closed).
        assert_eq!(Verdict::from_text("looks good to me"), Verdict::Reject);
        assert_eq!(Verdict::from_text(""), Verdict::Reject);
    }

    #[test]
    fn verdict_prefers_with_changes_over_plain_approve() {
        // The string contains both "approve" and "approve-with-changes" — the
        // more specific phrasing should win.
        let text = "Verdict: approve-with-changes — see findings below. approve.";
        assert_eq!(Verdict::from_text(text), Verdict::ApproveWithChanges);
    }

    #[test]
    fn extract_system_prompt_pulls_indented_block_scalar() {
        let audit = r#"
preface
```yaml
name: cto
role: Chief Technology Officer
system_prompt: |
  You are the CTO.
  Your job is to ship.

  Rule 1: emit JSON.
fallback_directive: |
  on retry, fix.
```
"#;
        let body = extract_system_prompt_from_audit(audit).unwrap();
        assert!(body.starts_with("You are the CTO."));
        assert!(body.contains("Rule 1: emit JSON."));
        // Should NOT include the next field
        assert!(!body.contains("fallback_directive"));
    }

    #[test]
    fn extract_errs_when_no_yaml_block() {
        let audit = "just plain markdown no yaml here";
        let err = extract_system_prompt_from_audit(audit).unwrap_err();
        assert!(err.to_string().contains("no fenced"));
    }

    #[test]
    fn provider_class_diverges_for_red_blue_defaults() {
        // The CLI's improve path runs adversarial-red on claude-sonnet-4-6
        // (Anthropic) and adversarial-blue on devstral-small-2:24b (Ollama).
        // These MUST map to different provider classes or the gated
        // reducer rejects every apply.
        let red = provider_class_of("claude-sonnet-4-6");
        let blue = provider_class_of("devstral-small-2:24b");
        assert_ne!(red, blue);
        assert_eq!(red, "anthropic");
        assert_eq!(blue, "ollama");
    }

    #[test]
    fn provider_class_unknown_for_bare_strings() {
        // No prefix and no `:` separator — unknown, which makes the
        // gated reducer reject (caller must pass a known provider).
        assert_eq!(provider_class_of("custom-rolled-model"), "unknown");
    }
}
