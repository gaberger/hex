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
    }
}

fn stdb_host() -> String {
    std::env::var("HEX_STDB_HOST").unwrap_or_else(|_| DEFAULT_STDB_HOST.to_string())
}
fn hex_db() -> String {
    std::env::var("HEX_STDB_HEXFLO_DB").unwrap_or_else(|_| DEFAULT_HEX_DB.to_string())
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
