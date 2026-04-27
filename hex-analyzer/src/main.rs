//! `hex-analyzer` — CLI surface for architectural-health detectors.
//!
//! Each `--<detector>` flag selects one or more analyzers; results are
//! merged into a single `{findings: [...]}` envelope. The improver
//! consumes this with `--json` (jq-friendly schema, top-level array).

use clap::Parser;
use serde_json::Value;

use hex_analyzer::analyzers::cohesion;
use hex_analyzer::analyzers::orphan::{self, OrphanOptions};

#[derive(Parser, Debug)]
#[command(
    name = "hex-analyzer",
    about = "Architectural-health detectors for hexagonal codebases",
    version
)]
struct Cli {
    /// Project root to analyze (defaults to current dir).
    #[arg(default_value = ".")]
    path: String,

    /// Report adapters that implement a port but are not bound in any
    /// composition-root file.
    #[arg(long = "orphan-adapters")]
    orphan_adapters: bool,

    /// Report port traits that have no `impl Port for Adapter` block
    /// anywhere in the workspace.
    #[arg(long = "orphan-ports")]
    orphan_ports: bool,

    /// Report port traits whose method shape suggests they bundle
    /// multiple unrelated concerns (kitchen-sink ports). Flagged when
    /// method count exceeds the configured threshold OR clusters share
    /// no parameter-type vocabulary.
    #[arg(long = "port-cohesion")]
    port_cohesion: bool,

    /// Emit JSON instead of human-readable output. The schema is
    /// `{findings: [{kind, ...}]}` — fields after `kind` vary per detector.
    #[arg(long)]
    json: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = orphan::resolve_root(&cli.path);

    // No detector flag → run everything (humans calling directly want
    // the full picture; the improver always passes a specific flag).
    let any_flag = cli.orphan_adapters || cli.orphan_ports || cli.port_cohesion;
    let want_orphans = cli.orphan_adapters || cli.orphan_ports || !any_flag;
    let want_cohesion = cli.port_cohesion || !any_flag;

    let mut findings: Vec<Value> = Vec::new();

    if want_orphans {
        let opts = OrphanOptions {
            orphan_adapters: cli.orphan_adapters || !any_flag,
            orphan_ports: cli.orphan_ports || !any_flag,
        };
        let report = orphan::analyze(&root, opts)?;
        for f in report.findings {
            findings.push(serde_json::to_value(f)?);
        }
    }

    if want_cohesion {
        let report = cohesion::analyze(&root)?;
        for f in report.findings {
            findings.push(serde_json::to_value(f)?);
        }
    }

    if cli.json {
        let envelope = serde_json::json!({ "findings": findings });
        println!("{}", serde_json::to_string_pretty(&envelope)?);
    } else {
        print_findings_human(&findings);
    }

    Ok(())
}

fn print_findings_human(findings: &[Value]) {
    if findings.is_empty() {
        println!("No findings.");
        return;
    }
    println!("{} finding(s):", findings.len());
    for f in findings {
        let kind = f.get("kind").and_then(Value::as_str).unwrap_or("?");
        let port = f.get("port").and_then(Value::as_str).unwrap_or("?");
        let file = f.get("file").and_then(Value::as_str).unwrap_or("?");
        let line = f.get("line").and_then(Value::as_u64).unwrap_or(0);
        match kind {
            "orphan_adapter" => {
                let adapter = f.get("adapter").and_then(Value::as_str).unwrap_or("?");
                println!("  [{kind}] {port} → {adapter} ({file}:{line})");
            }
            "port_cohesion" => {
                let count = f.get("method_count").and_then(Value::as_u64).unwrap_or(0);
                let n_clusters = f
                    .get("clusters")
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0);
                println!(
                    "  [{kind}] {port} ({file}:{line}) — {count} method(s) in {n_clusters} cluster(s)"
                );
            }
            _ => {
                println!("  [{kind}] {port} ({file}:{line})");
            }
        }
    }
}
