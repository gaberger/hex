//! `hex-analyzer` — CLI surface for architectural-health detectors.
//!
//! Each `--<detector>` flag selects one or more analyzers; results are
//! merged into a single `{findings: [...]}` envelope. The improver
//! consumes this with `--json` (jq-friendly schema, top-level array).

use clap::Parser;

use hex_analyzer::analyzers::orphan::{self, OrphanOptions, OrphanReport};

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

    /// Emit JSON instead of human-readable output. The schema is
    /// `{findings: [{kind, port, adapter?, file, line}]}`.
    #[arg(long)]
    json: bool,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let root = orphan::resolve_root(&cli.path);

    // If no detector flag is given, default to running both — the
    // improver invokes us per-flag, but humans calling this directly
    // probably want everything.
    let want_orphans = cli.orphan_adapters || cli.orphan_ports;
    let orphan_opts = OrphanOptions {
        orphan_adapters: cli.orphan_adapters || !want_orphans,
        orphan_ports: cli.orphan_ports || !want_orphans,
    };

    let report = orphan::analyze(&root, orphan_opts)?;

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_report_human(&report);
    }

    Ok(())
}

fn print_report_human(report: &OrphanReport) {
    if report.findings.is_empty() {
        println!("No findings.");
        return;
    }
    println!("{} finding(s):", report.findings.len());
    for f in &report.findings {
        match &f.adapter {
            Some(a) => println!("  [{}] {} → {} ({}:{})", f.kind, f.port, a, f.file, f.line),
            None => println!("  [{}] {} ({}:{})", f.kind, f.port, f.file, f.line),
        }
    }
}
