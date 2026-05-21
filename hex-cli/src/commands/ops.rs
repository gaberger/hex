//! Operator-grade primitives that close the gap between "the autonomous
//! SOP loop can do X" and "the operator can do X without raw curl".
//!
//! Every operation here was, until this module shipped, only reachable
//! by POSTing to a nexus REST URL by hand. The autonomous loop
//! (drafter → twin → executor → autonomous commit) is the consumer of
//! those endpoints internally; the operator should reach the same
//! endpoints through `hex ops <verb>` so the CLI is a peer of the loop,
//! not a separate world.
//!
//! Verbs:
//!
//!   hex ops write <path> --content <text>      # short content inline
//!   hex ops write <path> --file <localfile>    # whole-file content from disk
//!   hex ops send <to> [--subject <s>] --content <text>   # board ask
//!   hex ops abandon <commitment-id> [--reason <r>]       # close a wedged commitment
//!
//! All three go through proposed_by="operator-passthrough" or its sibling
//! endpoints, so the twin's source-guard exception applies + the
//! executor's cargo_check gate (for .rs paths) still runs + the autonomous
//! commit step lands the artifact on main. Operator never types `curl`.

use clap::Subcommand;
use std::path::PathBuf;

const NEXUS_DEFAULT: &str = "http://127.0.0.1:5555";
const STDB_DEFAULT: &str = "http://127.0.0.1:3033";
const STDB_DB_DEFAULT: &str = "hex";

#[derive(Debug, Subcommand)]
pub enum OpsAction {
    /// Write a file via the autonomous SOP loop. The bytes you provide
    /// land at <path> in the working tree after the twin auto-approves
    /// (operator-passthrough fast path), the executor verifies (cargo_check
    /// for .rs), and the autonomous commit step commits to main with
    /// `Co-Authored-By: hex-autonomous`.
    ///
    /// Replaces the raw POST /v1/database/hex/call/proposed_action_open
    /// + JSON payload incantation that operators were typing by hand.
    Write {
        /// Repo-relative destination path (e.g. examples/foo/src/lib.rs).
        path: String,
        /// Inline content. Mutually exclusive with --file.
        #[arg(long, conflicts_with = "file")]
        content: Option<String>,
        /// Read content from a local file. Mutually exclusive with --content.
        #[arg(long, conflicts_with = "content")]
        file: Option<PathBuf>,
        /// STDB host (default http://127.0.0.1:3033).
        #[arg(long, default_value = STDB_DEFAULT)]
        stdb: String,
        /// STDB database name (default hex).
        #[arg(long, default_value = STDB_DB_DEFAULT)]
        database: String,
    },

    /// Send a board ask to a persona via /api/org/send-message. Replaces
    /// the raw curl invocations operators were using to enqueue persona
    /// work over the SOP path.
    Send {
        /// Target persona (e.g. cto, ciso, cpo). @mentions in `content`
        /// also route — but `to` is the structured target for the org
        /// hierarchy routing.
        to: String,
        /// Message body. Use literal-content briefs (e.g. "write FOO
        /// containing only one line: BAR") to bypass the LLM via the
        /// drafter's shortcut.
        #[arg(long)]
        content: String,
        /// Optional subject line.
        #[arg(long, default_value = "")]
        subject: String,
        /// Nexus URL (default http://127.0.0.1:5555).
        #[arg(long, default_value = NEXUS_DEFAULT)]
        nexus: String,
    },

    /// Read DMs received by an agent (default: operator). Symmetric
    /// counterpart to `hex ops send` — replaces the raw
    /// `hex stdb query --db agent-comms "SELECT * FROM agent_messages …"`
    /// that operators were typing by hand to check persona replies.
    Read {
        /// Recipient to read (e.g. operator, ceo). Defaults to operator
        /// so the common case ("did the team reply yet?") is one word.
        #[arg(long, default_value = "operator")]
        agent: String,
        /// Max messages to return. Default 20 — enough to see the
        /// last wave of board asks + replies in one screenful.
        #[arg(long, default_value_t = 20)]
        limit: u32,
        /// Only show messages from this sender. Useful for
        /// `hex ops read --from cto` to see one persona's stream.
        #[arg(long)]
        from: Option<String>,
        /// Print full message body instead of truncating to 200 chars.
        #[arg(long)]
        full: bool,
        /// Output as JSON instead of formatted text.
        #[arg(long)]
        json: bool,
        /// Nexus URL (default http://127.0.0.1:5555).
        #[arg(long, default_value = NEXUS_DEFAULT)]
        nexus: String,
    },

    /// Abandon an open commitment with a reason. Use when the SOP path
    /// has wedged on a commitment the operator no longer wants (typo'd
    /// path, stale brief, etc.).
    Abandon {
        /// Commitment ID (from `hex inbox list` or `hex chat history`).
        commitment_id: u64,
        /// Human-readable reason recorded with the abandon event.
        #[arg(long, default_value = "operator: abandoned via hex ops abandon")]
        reason: String,
        /// STDB host (default http://127.0.0.1:3033).
        #[arg(long, default_value = STDB_DEFAULT)]
        stdb: String,
        /// STDB database name (default hex).
        #[arg(long, default_value = STDB_DB_DEFAULT)]
        database: String,
    },
}

pub async fn run(action: OpsAction) -> anyhow::Result<()> {
    match action {
        OpsAction::Write {
            path,
            content,
            file,
            stdb,
            database,
        } => write(path, content, file, stdb, database).await,
        OpsAction::Send {
            to,
            content,
            subject,
            nexus,
        } => send(to, subject, content, nexus).await,
        OpsAction::Read {
            agent,
            limit,
            from,
            full,
            json,
            nexus,
        } => read(agent, limit, from, full, json, nexus).await,
        OpsAction::Abandon {
            commitment_id,
            reason,
            stdb,
            database,
        } => abandon(commitment_id, reason, stdb, database).await,
    }
}

async fn write(
    path: String,
    content: Option<String>,
    file: Option<PathBuf>,
    stdb: String,
    database: String,
) -> anyhow::Result<()> {
    let body_text = match (content, file) {
        (Some(c), None) => c,
        (None, Some(f)) => {
            std::fs::read_to_string(&f)
                .map_err(|e| anyhow::anyhow!("read --file {}: {}", f.display(), e))?
        }
        (None, None) => {
            anyhow::bail!("hex ops write: provide one of --content <text> or --file <path>")
        }
        (Some(_), Some(_)) => unreachable!("clap conflicts_with enforces this"),
    };

    if path.is_empty() {
        anyhow::bail!("hex ops write: path is required");
    }
    if body_text.len() > 24 * 1024 {
        anyhow::bail!(
            "hex ops write: content is {} bytes (cap is 24 KB per drafter CONTENT_CAP_BYTES)",
            body_text.len()
        );
    }

    let payload = serde_json::json!({
        "path": path,
        "content": body_text,
    });
    // proposed_action_open([kind, payload_json, proposed_by, commitment_id])
    let req_body = serde_json::json!([
        "file_write",
        payload.to_string(),
        "operator-passthrough",
        0u64,
    ]);

    let url = format!(
        "{}/v1/database/{}/call/proposed_action_open",
        stdb.trim_end_matches('/'),
        database
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let resp = client.post(&url).json(&req_body).send().await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "hex ops write: nexus REST returned {}: {}\nURL: {}",
            status,
            body.chars().take(400).collect::<String>(),
            url
        );
    }
    println!(
        "{} {} ({} bytes) — proposed_action enqueued; twin will auto-approve, executor + autonomous commit will follow within a tick",
        path,
        if body.is_empty() { "ok" } else { "ok" },
        body_text.len()
    );
    Ok(())
}

async fn send(
    to: String,
    subject: String,
    content: String,
    nexus: String,
) -> anyhow::Result<()> {
    if to.is_empty() || content.is_empty() {
        anyhow::bail!("hex ops send: <to> and --content are both required");
    }
    let url = format!(
        "{}/api/org/send-message",
        nexus.trim_end_matches('/')
    );
    let req_body = serde_json::json!({
        "to": to,
        "from": "operator",
        "subject": subject,
        "content": content,
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let resp = client.post(&url).json(&req_body).send().await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "hex ops send: nexus returned {}: {}",
            status,
            body.chars().take(400).collect::<String>()
        );
    }
    println!("{}", body);
    Ok(())
}

async fn read(
    agent: String,
    limit: u32,
    from: Option<String>,
    full: bool,
    json_out: bool,
    nexus: String,
) -> anyhow::Result<()> {
    let url = format!(
        "{}/api/org/messages?agent={}&limit={}",
        nexus.trim_end_matches('/'),
        urlencode(&agent),
        limit
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let resp = client.get(&url).send().await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "hex ops read: nexus returned {}: {}",
            status,
            body.chars().take(400).collect::<String>()
        );
    }

    if json_out {
        println!("{}", body);
        return Ok(());
    }

    // Parse + pretty-print. Falls back to raw JSON on parse failure so
    // operators never lose data — uglier output but always works.
    let parsed: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => {
            println!("{}", body);
            return Ok(());
        }
    };

    let messages = parsed
        .get("messages")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();

    if messages.is_empty() {
        println!("⬡ no messages for {agent}");
        return Ok(());
    }

    println!(
        "⬡ {} message(s) for {}",
        messages.len(),
        agent
    );
    println!();

    for msg in &messages {
        let sender = msg.get("from").and_then(|v| v.as_str()).unwrap_or("?");
        let when = msg
            .get("timestamp")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let content = msg
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        // Take the timestamp's HH:MM:SS portion for compact display.
        let ts = when.split('T').nth(1).and_then(|s| s.split('.').next()).unwrap_or(when);
        // Skip if --from filter set and sender doesn't match.
        if let Some(ref f) = from {
            if sender != f.as_str() { continue; }
        }
        let body_text = if full || content.len() <= 200 {
            content.trim().to_string()
        } else {
            format!("{}…", &content[..200].trim_end())
        };
        println!("─── {} @ {} ──────────────────", sender, ts);
        println!("{}", body_text);
        println!();
    }
    Ok(())
}

/// Minimal URL-encode for query-string values. Just handles space + & +
/// = + # + ? — enough for agent role names which are kebab-case ASCII.
fn urlencode(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
            ' ' => "%20".to_string(),
            _ => format!("%{:02X}", c as u32),
        })
        .collect()
}

async fn abandon(
    commitment_id: u64,
    reason: String,
    stdb: String,
    database: String,
) -> anyhow::Result<()> {
    let url = format!(
        "{}/v1/database/{}/call/commitment_abandon",
        stdb.trim_end_matches('/'),
        database
    );
    let req_body = serde_json::json!([commitment_id, reason]);
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;
    let resp = client.post(&url).json(&req_body).send().await?;
    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!(
            "hex ops abandon #{}: nexus returned {}: {}",
            commitment_id,
            status,
            body.chars().take(400).collect::<String>()
        );
    }
    println!("commitment #{} abandoned: {}", commitment_id, reason);
    Ok(())
}
