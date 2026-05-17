//! `hex fs` — native filesystem primitives (ADR-2604142100, wp-hex-native-filesystem P2.1).
//!
//! Thin dispatcher on top of the `/api/fs/*` endpoints in hex-nexus. Provides
//! eight subcommands — `list`, `read`, `search`, `glob`, `tree`, `stat`,
//! `head`, `tail` — so agents and humans can query the project filesystem
//! without reaching for Bash, Read, Grep, or Glob.
//!
//! Output defaults to plain text; pass `--json` to emit the raw nexus response.
//! When nexus truncates a paginated response, the plain-text renderer prints
//! `… truncated <shown>/<total>` so callers know they need to page.

use clap::Subcommand;
use colored::Colorize;
use serde_json::{Value, json};

use crate::nexus_client::NexusClient;

#[derive(Subcommand)]
pub enum FsAction {
    /// List directory entries (optional glob filter)
    List {
        /// Path relative to the project root
        #[arg(default_value = ".")]
        path: String,
        /// Glob pattern applied to entry names (e.g. '*.toml')
        #[arg(long)]
        pattern: Option<String>,
        /// Maximum number of entries to return
        #[arg(long)]
        limit: Option<usize>,
        /// Entry offset (0-based)
        #[arg(long)]
        offset: Option<usize>,
        /// Emit raw JSON
        #[arg(long)]
        json: bool,
    },
    /// Read file contents with line-level pagination
    Read {
        /// File path relative to the project root
        path: String,
        /// Zero-based starting line
        #[arg(long)]
        offset: Option<usize>,
        /// Maximum number of lines (default 500)
        #[arg(long)]
        lines: Option<usize>,
        /// Emit raw JSON
        #[arg(long)]
        json: bool,
    },
    /// Ripgrep-style regex content search
    Search {
        /// Directory to search under (relative to project root)
        #[arg(default_value = ".")]
        path: String,
        /// Regex pattern
        pattern: String,
        /// `ignore`/ripgrep file-type filter (e.g. rust, toml, js)
        #[arg(long = "type", value_name = "TYPE")]
        file_type: Option<String>,
        /// Case-insensitive match
        #[arg(long, short = 'i')]
        case_insensitive: bool,
        /// Maximum matches returned
        #[arg(long)]
        limit: Option<usize>,
        /// Match offset
        #[arg(long)]
        offset: Option<usize>,
        /// Emit raw JSON
        #[arg(long)]
        json: bool,
    },
    /// Walk + glob-match file paths (supports `**/*.rs`)
    Glob {
        /// Directory root (relative to project root)
        #[arg(default_value = ".")]
        path: String,
        /// Glob pattern
        pattern: String,
        /// Maximum paths returned
        #[arg(long)]
        limit: Option<usize>,
        /// Offset
        #[arg(long)]
        offset: Option<usize>,
        /// Emit raw JSON
        #[arg(long)]
        json: bool,
    },
    /// Bounded-depth directory tree
    Tree {
        /// Directory (relative to project root)
        #[arg(default_value = ".")]
        path: String,
        /// Maximum depth (default 3, capped at 16)
        #[arg(long)]
        depth: Option<usize>,
        /// Maximum nodes returned
        #[arg(long)]
        limit: Option<usize>,
        /// Emit raw JSON
        #[arg(long)]
        json: bool,
    },
    /// File metadata (kind, size, readonly, mtime)
    Stat {
        /// Path relative to the project root
        path: String,
        /// Emit raw JSON
        #[arg(long)]
        json: bool,
    },
    /// First N lines of a file (default 20)
    Head {
        /// File path relative to the project root
        path: String,
        /// Number of lines
        #[arg(long, short = 'n')]
        lines: Option<usize>,
        /// Emit raw JSON
        #[arg(long)]
        json: bool,
    },
    /// Last N lines of a file (default 20)
    Tail {
        /// File path relative to the project root
        path: String,
        /// Number of lines
        #[arg(long, short = 'n')]
        lines: Option<usize>,
        /// Emit raw JSON
        #[arg(long)]
        json: bool,
    },
}

pub async fn run(action: FsAction) -> anyhow::Result<()> {
    let nexus = NexusClient::from_env();
    nexus.ensure_running().await?;

    match action {
        FsAction::List { path, pattern, limit, offset, json } => {
            let url = build_query(
                "/api/fs/list",
                &[
                    ("path", Some(path.as_str())),
                    ("pattern", pattern.as_deref()),
                    ("limit", limit.map(|n| n.to_string()).as_deref()),
                    ("offset", offset.map(|n| n.to_string()).as_deref()),
                ],
            );
            let resp = nexus.get(&url).await?;
            if json { print_json(&resp) } else { render_list(&resp) };
            Ok(())
        }
        FsAction::Read { path, offset, lines, json } => {
            let url = build_query(
                "/api/fs/read",
                &[
                    ("path", Some(path.as_str())),
                    ("offset", offset.map(|n| n.to_string()).as_deref()),
                    ("lines", lines.map(|n| n.to_string()).as_deref()),
                ],
            );
            let resp = nexus.get(&url).await?;
            if json { print_json(&resp) } else { render_read(&resp) };
            Ok(())
        }
        FsAction::Search { path, pattern, file_type, case_insensitive, limit, offset, json } => {
            let mut body = json!({
                "path": path,
                "pattern": pattern,
                "caseInsensitive": case_insensitive,
            });
            if let Some(t) = file_type { body["type"] = Value::String(t); }
            if let Some(n) = limit { body["limit"] = Value::from(n); }
            if let Some(n) = offset { body["offset"] = Value::from(n); }
            let resp = nexus.post("/api/fs/search", &body).await?;
            if json { print_json(&resp) } else { render_search(&resp) };
            Ok(())
        }
        FsAction::Glob { path, pattern, limit, offset, json } => {
            let url = build_query(
                "/api/fs/glob",
                &[
                    ("path", Some(path.as_str())),
                    ("pattern", Some(pattern.as_str())),
                    ("limit", limit.map(|n| n.to_string()).as_deref()),
                    ("offset", offset.map(|n| n.to_string()).as_deref()),
                ],
            );
            let resp = nexus.get(&url).await?;
            if json { print_json(&resp) } else { render_glob(&resp) };
            Ok(())
        }
        FsAction::Tree { path, depth, limit, json } => {
            let url = build_query(
                "/api/fs/tree",
                &[
                    ("path", Some(path.as_str())),
                    ("depth", depth.map(|n| n.to_string()).as_deref()),
                    ("limit", limit.map(|n| n.to_string()).as_deref()),
                ],
            );
            let resp = nexus.get(&url).await?;
            if json { print_json(&resp) } else { render_tree(&resp) };
            Ok(())
        }
        FsAction::Stat { path, json } => {
            let url = build_query("/api/fs/stat", &[("path", Some(path.as_str()))]);
            let resp = nexus.get(&url).await?;
            if json { print_json(&resp) } else { render_stat(&resp) };
            Ok(())
        }
        FsAction::Head { path, lines, json } => {
            let url = build_query(
                "/api/fs/head",
                &[
                    ("path", Some(path.as_str())),
                    ("lines", lines.map(|n| n.to_string()).as_deref()),
                ],
            );
            let resp = nexus.get(&url).await?;
            if json { print_json(&resp) } else { render_head_tail(&resp) };
            Ok(())
        }
        FsAction::Tail { path, lines, json } => {
            let url = build_query(
                "/api/fs/tail",
                &[
                    ("path", Some(path.as_str())),
                    ("lines", lines.map(|n| n.to_string()).as_deref()),
                ],
            );
            let resp = nexus.get(&url).await?;
            if json { print_json(&resp) } else { render_head_tail(&resp) };
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// URL helpers
// ---------------------------------------------------------------------------

fn build_query(path: &str, params: &[(&str, Option<&str>)]) -> String {
    let mut first = true;
    let mut out = String::from(path);
    for (k, v) in params {
        if let Some(val) = v {
            out.push(if first { '?' } else { '&' });
            first = false;
            out.push_str(k);
            out.push('=');
            out.push_str(&urlencode(val));
        }
    }
    out
}

/// Minimal RFC-3986 query-string escaper — we only need it for user-supplied
/// paths and glob patterns, not a full URL parser.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Renderers — plain-text output for humans. `--json` bypasses these.
// ---------------------------------------------------------------------------

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string()));
}

fn truncated_note(shown: usize, total: usize, truncated: bool) {
    if truncated {
        eprintln!(
            "{} truncated {}/{}",
            "…".yellow(),
            shown,
            total
        );
    }
}

fn render_list(resp: &Value) {
    let entries = resp.get("entries").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let total = resp.get("total").and_then(|v| v.as_u64()).unwrap_or(entries.len() as u64);
    let truncated = resp.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
    for e in &entries {
        let name = e.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let kind = e.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let size = e.get("size").and_then(|v| v.as_u64());
        let size_str = size.map(|s| format!("{:>10}", s)).unwrap_or_else(|| "          ".into());
        let marker = match kind {
            "dir" => "d".blue(),
            "symlink" => "l".magenta(),
            "file" => "f".normal(),
            _ => "?".dimmed(),
        };
        println!("  {marker}  {size_str}  {name}");
    }
    truncated_note(entries.len(), total as usize, truncated);
}

fn render_read(resp: &Value) {
    let content = resp.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let offset = resp.get("offset").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let total = resp.get("totalLines").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
    let truncated = resp.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
    println!("{}", content);
    let shown = content.split('\n').count();
    if truncated {
        eprintln!(
            "{} truncated lines {}..{}/{}",
            "…".yellow(),
            offset,
            offset + shown,
            total
        );
    }
}

fn render_search(resp: &Value) {
    let matches = resp.get("matches").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let total = resp.get("total").and_then(|v| v.as_u64()).unwrap_or(matches.len() as u64);
    let truncated = resp.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
    for m in &matches {
        let path = m.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let line = m.get("line").and_then(|v| v.as_u64()).unwrap_or(0);
        let text = m.get("text").and_then(|v| v.as_str()).unwrap_or("");
        println!("{}:{}:{}", path.cyan(), line.to_string().yellow(), text);
    }
    truncated_note(matches.len(), total as usize, truncated);
}

fn render_glob(resp: &Value) {
    let paths = resp.get("paths").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let total = resp.get("total").and_then(|v| v.as_u64()).unwrap_or(paths.len() as u64);
    let truncated = resp.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
    for p in &paths {
        if let Some(s) = p.as_str() {
            println!("{}", s);
        }
    }
    truncated_note(paths.len(), total as usize, truncated);
}

fn render_tree(resp: &Value) {
    let nodes = resp.get("nodes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let total = resp.get("total").and_then(|v| v.as_u64()).unwrap_or(nodes.len() as u64);
    let truncated = resp.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false);
    for n in &nodes {
        let depth = n.get("depth").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
        let name = n.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let kind = n.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let indent = "  ".repeat(depth);
        let suffix = if kind == "dir" { "/" } else { "" };
        let painted = if kind == "dir" { name.blue().to_string() } else { name.to_string() };
        println!("{indent}{painted}{suffix}");
    }
    truncated_note(nodes.len(), total as usize, truncated);
}

fn render_stat(resp: &Value) {
    let path = resp.get("path").and_then(|v| v.as_str()).unwrap_or("");
    let kind = resp.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let size = resp.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
    let readonly = resp.get("readonly").and_then(|v| v.as_bool()).unwrap_or(false);
    let modified = resp.get("modifiedUnix").and_then(|v| v.as_i64());
    println!("{} {}", "path:".dimmed(), path);
    println!("{} {}", "kind:".dimmed(), kind);
    println!("{} {}", "size:".dimmed(), size);
    println!("{} {}", "readonly:".dimmed(), readonly);
    if let Some(m) = modified {
        println!("{} {}", "modified_unix:".dimmed(), m);
    }
}

fn render_head_tail(resp: &Value) {
    let content = resp.get("content").and_then(|v| v.as_str()).unwrap_or("");
    let total = resp.get("totalLines").and_then(|v| v.as_u64()).unwrap_or(0);
    let returned = resp.get("returnedLines").and_then(|v| v.as_u64()).unwrap_or(0);
    println!("{}", content);
    if total > returned {
        eprintln!("{} showing {}/{} lines", "…".yellow(), returned, total);
    }
}
