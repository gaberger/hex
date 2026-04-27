//! `hex-analyzer` — CLI surface for architectural-health detectors.
//!
//! Each `--<detector>` flag selects one or more analyzers; results are
//! merged into a single `{findings: [...]}` envelope. The improver
//! consumes this with `--json` (jq-friendly schema, top-level array).

use clap::Parser;
use serde_json::Value;

use hex_analyzer::analyzers::cohesion;
use hex_analyzer::analyzers::composition_churn;
use hex_analyzer::analyzers::dead_layer;
use hex_analyzer::analyzers::duplication;
use hex_analyzer::analyzers::god_types::{self, GodTypeThresholds};
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

    /// Report struct/enum types under `domain/` whose total declaration
    /// + impl LOC exceeds 300 OR whose impl blocks expose more than 10
    /// public methods. Override defaults via `.hex/project.json` →
    /// `analyzer.god_type` (`loc_threshold`, `public_methods_threshold`).
    #[arg(long = "god-types")]
    god_types: bool,

    /// Report pairs of `impl Port for Adapter` blocks whose token
    /// bodies overlap by ≥ 0.6 (multiset Jaccard on tree-sitter leaf
    /// tokens). Two adapters doing the same thing behind one contract.
    #[arg(long = "adapter-duplication")]
    adapter_duplication: bool,

    /// Report layer directories (`domain/`, `ports/`, `usecases/`,
    /// `adapters/secondary/`) that have zero inbound `use` references
    /// from elsewhere in the workspace — code that nothing calls.
    #[arg(long = "dead-layers")]
    dead_layers: bool,

    /// Report composition drift: ratio of commits touching wiring
    /// (`*composition-root*`, `*compose*.rs`, `*lib.rs`) to ADRs
    /// accepted in the same window. Flags when the ratio exceeds
    /// 1.5 — wiring is being rewritten faster than decisions are
    /// being recorded. Window is controlled by `--window`.
    #[arg(long = "composition-churn")]
    composition_churn: bool,

    /// Window for the `--composition-churn` detector. Accepts
    /// `Nh` / `Nd` / `Nw` (e.g. `24h`, `30d`, `2w`). Forwarded to
    /// `git log --since="N <unit> ago"` and used for the ADR-date
    /// cutoff.
    #[arg(long = "window", default_value = composition_churn::DEFAULT_WINDOW)]
    window: String,

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
    let any_flag = cli.orphan_adapters
        || cli.orphan_ports
        || cli.port_cohesion
        || cli.god_types
        || cli.adapter_duplication
        || cli.dead_layers
        || cli.composition_churn;
    let want_orphans = cli.orphan_adapters || cli.orphan_ports || !any_flag;
    let want_cohesion = cli.port_cohesion || !any_flag;
    let want_god_types = cli.god_types || !any_flag;
    let want_duplication = cli.adapter_duplication || !any_flag;
    let want_dead_layers = cli.dead_layers || !any_flag;
    let want_composition_churn = cli.composition_churn || !any_flag;

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

    if want_god_types {
        let thresholds = GodTypeThresholds::from_project_root(&root);
        let report = god_types::analyze(&root, thresholds)?;
        for f in report.findings {
            findings.push(serde_json::to_value(f)?);
        }
    }

    if want_duplication {
        let report = duplication::analyze(&root)?;
        for f in report.findings {
            findings.push(serde_json::to_value(f)?);
        }
    }

    if want_dead_layers {
        let report = dead_layer::analyze(&root)?;
        for f in report.findings {
            findings.push(serde_json::to_value(f)?);
        }
    }

    if want_composition_churn {
        let report = composition_churn::analyze(&root, &cli.window)?;
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
        let file = f.get("file").and_then(Value::as_str).unwrap_or("?");
        match kind {
            "orphan_adapter" => {
                let port = f.get("port").and_then(Value::as_str).unwrap_or("?");
                let adapter = f.get("adapter").and_then(Value::as_str).unwrap_or("?");
                let line = f.get("line").and_then(Value::as_u64).unwrap_or(0);
                println!("  [{kind}] {port} → {adapter} ({file}:{line})");
            }
            "port_cohesion" => {
                let port = f.get("port").and_then(Value::as_str).unwrap_or("?");
                let line = f.get("line").and_then(Value::as_u64).unwrap_or(0);
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
            "god_type" => {
                let type_name = f.get("type").and_then(Value::as_str).unwrap_or("?");
                let lines = f.get("lines").and_then(Value::as_u64).unwrap_or(0);
                let methods = f
                    .get("public_methods")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                println!(
                    "  [{kind}] {type_name} ({file}) — {lines} LOC, {methods} public method(s)"
                );
            }
            "adapter_duplication" => {
                let port = f.get("port").and_then(Value::as_str).unwrap_or("?");
                let adapter_a = f.get("adapter_a").and_then(Value::as_str).unwrap_or("?");
                let adapter_b = f.get("adapter_b").and_then(Value::as_str).unwrap_or("?");
                let file_a = f.get("file_a").and_then(Value::as_str).unwrap_or("?");
                let line_a = f.get("line_a").and_then(Value::as_u64).unwrap_or(0);
                let file_b = f.get("file_b").and_then(Value::as_str).unwrap_or("?");
                let line_b = f.get("line_b").and_then(Value::as_u64).unwrap_or(0);
                let sim = f.get("similarity").and_then(Value::as_f64).unwrap_or(0.0);
                println!(
                    "  [{kind}] {port}: {adapter_a} ({file_a}:{line_a}) ≈ {adapter_b} ({file_b}:{line_b}) — Jaccard {sim:.2}"
                );
            }
            "dead_layer" => {
                let layer = f.get("layer").and_then(Value::as_str).unwrap_or("?");
                let layer_kind = f.get("layer_kind").and_then(Value::as_str).unwrap_or("?");
                println!("  [{kind}] {layer_kind} — {layer} (zero inbound references)");
            }
            "composition_churn" => {
                let window = f.get("window").and_then(Value::as_str).unwrap_or("?");
                let commits = f
                    .get("commits_touching_wiring")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let adrs = f
                    .get("accepted_adrs_in_window")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let ratio = f
                    .get("ratio")
                    .and_then(Value::as_f64)
                    .unwrap_or(f64::NAN);
                let touched = f
                    .get("files_touched")
                    .and_then(Value::as_array)
                    .map(|a| a.len())
                    .unwrap_or(0);
                println!(
                    "  [{kind}] window={window} — {commits} wiring commit(s) vs {adrs} accepted ADR(s) (ratio {ratio:.2}, {touched} file(s) touched)"
                );
            }
            _ => {
                let port = f.get("port").and_then(Value::as_str).unwrap_or("?");
                let line = f.get("line").and_then(Value::as_u64).unwrap_or(0);
                println!("  [{kind}] {port} ({file}:{line})");
            }
        }
    }
}
