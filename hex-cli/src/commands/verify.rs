//! `hex verify <claim>` — adversarial verification of a natural-language claim.
//!
//! Operator types a claim about the repo or system; we attempt to FALSIFY
//! it. Returns one of:
//!   CONFIRMED   — adversarial check ran, came up empty (no counter-evidence)
//!   REFUTED     — counter-evidence found; the claim is false
//!   INCONCLUSIVE — couldn't run a falsifiable check (claim too vague or
//!                  needs context we don't have)
//!
//! Two paths:
//!   1. Deterministic — common claim shapes (boundary check, file
//!      presence, secret absence, etc.) map to existing checks that can
//!      authoritatively pass/fail.
//!   2. LLM-driven — claims that don't match a deterministic check route
//!      through the adversarial-red persona via /api/inference/complete;
//!      verdict + evidence are parsed from the response.
//!
//! Builds on docs/adrs work earlier today: same adversarial-pattern that
//! verified 7 ADRs by running checks that COULD fail.

use anyhow::Result;
use clap::Parser;
use colored::Colorize;
use regex::Regex;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Parser)]
pub struct VerifyArgs {
    /// The claim to verify, in plain English.
    /// Examples:
    ///   hex verify "domain layer has zero boundary violations"
    ///   hex verify "no SQLite hub.db reference remains in code"
    ///   hex verify "all Accepted ADRs have implementation files"
    pub claim: Vec<String>,

    /// Emit JSON instead of human-readable output.
    #[arg(long)]
    pub json: bool,

    /// Skip the deterministic-check phase; force LLM-only.
    #[arg(long)]
    pub llm_only: bool,
}

#[derive(Debug)]
enum Verdict {
    Confirmed,
    Refuted,
    Inconclusive,
}

impl Verdict {
    fn as_str(&self) -> &'static str {
        match self {
            Verdict::Confirmed => "CONFIRMED",
            Verdict::Refuted => "REFUTED",
            Verdict::Inconclusive => "INCONCLUSIVE",
        }
    }
    fn colored(&self) -> colored::ColoredString {
        match self {
            Verdict::Confirmed => "CONFIRMED".green().bold(),
            Verdict::Refuted => "REFUTED".red().bold(),
            Verdict::Inconclusive => "INCONCLUSIVE".yellow().bold(),
        }
    }
}

struct CheckResult {
    verdict: Verdict,
    summary: String,
    evidence: Vec<String>,
    method: &'static str, // "deterministic" | "adversarial-llm"
}

pub async fn run(args: VerifyArgs) -> Result<()> {
    let claim = args.claim.join(" ").trim().to_string();
    if claim.is_empty() {
        anyhow::bail!("usage: hex verify <claim>");
    }

    if !args.json {
        println!("{} {}", "claim:".cyan(), claim);
        println!();
    }

    let result = if args.llm_only {
        verify_via_llm(&claim).await?
    } else {
        match try_deterministic(&claim) {
            Some(r) => r,
            None => verify_via_llm(&claim).await?,
        }
    };

    if args.json {
        let body = serde_json::json!({
            "claim": claim,
            "verdict": result.verdict.as_str(),
            "summary": result.summary,
            "evidence": result.evidence,
            "method": result.method,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        println!("  verdict:  {}", result.verdict.colored());
        println!("  method:   {}", result.method);
        println!("  summary:  {}", result.summary);
        if !result.evidence.is_empty() {
            println!("  evidence:");
            for ev in &result.evidence {
                println!("    - {}", ev);
            }
        }
    }

    // Exit code: 0 confirmed, 1 refuted, 2 inconclusive — operator can
    // script around it.
    std::process::exit(match result.verdict {
        Verdict::Confirmed => 0,
        Verdict::Refuted => 1,
        Verdict::Inconclusive => 2,
    });
}

// ── Deterministic checks ────────────────────────────────────────────────────
//
// Each check function:
//   - returns None if the claim doesn't match its pattern (try next check)
//   - returns Some(CheckResult) with verdict + evidence when it matched
//
// Add a new check by adding a function and calling it from try_deterministic.

fn try_deterministic(claim: &str) -> Option<CheckResult> {
    let lower = claim.to_lowercase();

    [
        check_boundary_violations as fn(&str, &str) -> Option<CheckResult>,
        check_secret_in_tree,
        check_no_substring,
        check_files_under,
        check_adr_status,
    ]
    .iter()
    .find_map(|f| f(claim, &lower))
}

/// Matches: "domain ... boundary violations" / "hexagonal layering"
fn check_boundary_violations(_claim: &str, lower: &str) -> Option<CheckResult> {
    if !(lower.contains("boundary") || lower.contains("hexagonal") || lower.contains("layering")) {
        return None;
    }
    if !(lower.contains("violation") || lower.contains("zero") || lower.contains("no ")) {
        return None;
    }

    let out = Command::new("hex").arg("analyze").arg(".").output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout);

    let zero_violations = s.contains("0 boundary violations");
    let grade = Regex::new(r"score (\d+)/100")
        .ok()
        .and_then(|re| re.captures(&s))
        .and_then(|c| c.get(1).map(|m| m.as_str().to_string()))
        .unwrap_or_else(|| "?".to_string());

    Some(CheckResult {
        verdict: if zero_violations { Verdict::Confirmed } else { Verdict::Refuted },
        summary: format!(
            "hex analyze . reports {} boundary violations (score {}/100)",
            if zero_violations { "0" } else { "≥1" },
            grade
        ),
        evidence: vec![
            "`hex analyze .` — full hexagonal layering check across the workspace".into(),
            format!("score {}/100", grade),
        ],
        method: "deterministic",
    })
}

/// Matches: "no secrets" / "no .env" / "no api key in"
fn check_secret_in_tree(_claim: &str, lower: &str) -> Option<CheckResult> {
    let mentions_secret = lower.contains("secret") || lower.contains(".env")
        || lower.contains("api key") || lower.contains("apikey");
    let negation = lower.contains("no ") || lower.contains("zero")
        || lower.contains("absent") || lower.contains("without");
    if !(mentions_secret && negation) {
        return None;
    }

    let out = Command::new("git")
        .args(["ls-files"])
        .output().ok()?;
    let files: Vec<&str> = std::str::from_utf8(&out.stdout).ok()?
        .lines().collect();
    let mut hits = Vec::new();
    for f in &files {
        if f.ends_with(".env") || f == &".env" || f.ends_with("/credentials.json") {
            hits.push(format!("tracked path: {}", f));
        }
    }
    Some(CheckResult {
        verdict: if hits.is_empty() { Verdict::Confirmed } else { Verdict::Refuted },
        summary: format!(
            "tracked tree scan: {} suspicious path(s) ({} total files)",
            hits.len(),
            files.len()
        ),
        evidence: if hits.is_empty() {
            vec!["no `.env`, `credentials.json` tracked".into()]
        } else { hits },
        method: "deterministic",
    })
}

/// Matches "no X in code" / "no X remains" — grep for literal token X.
fn check_no_substring(claim: &str, lower: &str) -> Option<CheckResult> {
    let re = Regex::new(r#"no [`"'']?([\w.\-/]{3,})[`"'']? (in|remains|present|exists)"#).ok()?;
    let cap = re.captures(&lower)?;
    let token = cap.get(1)?.as_str().to_string();
    // Skip if the deterministic-secret check already covers it
    if matches!(token.as_str(), "secret" | "secrets" | ".env" | "api" | "key") {
        return None;
    }

    let out = Command::new("git").args(["grep", "-l", "-E", &regex::escape(&token)])
        .output().ok()?;
    let body = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = body.lines().take(10).collect();
    let n = body.lines().count();

    Some(CheckResult {
        verdict: if n == 0 { Verdict::Confirmed } else { Verdict::Refuted },
        summary: format!(
            "git grep `{}`: {} file(s) match (claim was: {})",
            token, n, claim
        ),
        evidence: if n == 0 {
            vec![format!("`git grep -l {}` returned 0 hits", token)]
        } else {
            lines.iter().map(|l| format!("hit: {}", l)).collect()
        },
        method: "deterministic",
    })
}

/// Matches: "files under <dir>" / "X files in <dir>"
fn check_files_under(_claim: &str, lower: &str) -> Option<CheckResult> {
    let re = Regex::new(r"(?:files? (?:under|in) |present in )([\w./\-]+)").ok()?;
    let cap = re.captures(lower)?;
    let dir = cap.get(1)?.as_str();
    if !Path::new(dir).exists() {
        return None;
    }
    // Use git ls-files for an authoritative tracked-file count under dir.
    // Falls back to 0 if not in a git repo.
    let out = Command::new("git").args(["ls-files", "--", dir]).output().ok()?;
    let count = String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.is_empty()).count();
    Some(CheckResult {
        verdict: if count > 0 { Verdict::Confirmed } else { Verdict::Refuted },
        summary: format!("{} contains {} file(s)", dir, count),
        evidence: vec![format!("walked `{}` — {} regular files", dir, count)],
        method: "deterministic",
    })
}

/// Matches: "all <status> ADRs have <something>" / "<N> Proposed ADRs"
fn check_adr_status(_claim: &str, lower: &str) -> Option<CheckResult> {
    if !lower.contains("adr") { return None; }
    let entries = std::fs::read_dir("docs/adrs").ok()?;
    let mut totals = std::collections::HashMap::<String, u32>::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("md") { continue; }
        if let Ok(text) = std::fs::read_to_string(&p) {
            // Handle both `**Status:** Accepted` and `Status: Accepted` (with
            // bold variants). The trailing `\*\*` after the colon was missing
            // from the first cut → 0 ADRs matched. Use (?m) so ^ matches each line.
            let re = Regex::new(r"(?im)^\s*\*{0,2}Status\*{0,2}:\s*\*{0,2}\s*(\w+)").ok()?;
            if let Some(c) = re.captures(&text) {
                let s = c.get(1)?.as_str().to_lowercase();
                let bucket = match s.as_str() {
                    "accepted" => "accepted",
                    "proposed" => "proposed",
                    "rejected" => "rejected",
                    "superseded" => "superseded",
                    "deprecated" => "deprecated",
                    _ => "other",
                };
                *totals.entry(bucket.to_string()).or_insert(0) += 1;
            }
        }
    }
    let summary = {
        let mut parts: Vec<(String, u32)> = totals.iter().map(|(k,v)| (k.clone(), *v)).collect();
        parts.sort_by(|a,b| b.1.cmp(&a.1));
        parts.into_iter().map(|(k,v)| format!("{} {}", v, k)).collect::<Vec<_>>().join(", ")
    };
    Some(CheckResult {
        verdict: Verdict::Inconclusive,
        summary: format!("ADR census: {}", summary),
        evidence: vec!["docs/adrs/ scanned for Status: field".into()],
        method: "deterministic",
    })
}

// ── LLM-driven adversarial verification ─────────────────────────────────────

async fn verify_via_llm(claim: &str) -> Result<CheckResult> {
    let port: u16 = std::env::var("HEX_PORT").ok().and_then(|s| s.parse().ok()).unwrap_or(5555);
    let inference_url = format!("http://127.0.0.1:{}/api/inference/complete", port);

    let system = "You are an adversarial verifier evaluating a claim about a software repository. \
                  Output EXACTLY THREE LINES in this format and nothing else:\n\n\
                  VERDICT: <CONFIRMED or REFUTED or INCONCLUSIVE>\n\
                  SUMMARY: <one-line reason>\n\
                  EVIDENCE: <file path / ADR id / command name / or the word unknown>\n\n\
                  Use CONFIRMED only if you have actively considered how the claim could be false \
                  and found no counter-example. Use REFUTED if you can name a specific counter-example. \
                  Use INCONCLUSIVE if the claim is vague or you lack the data to check.\n\n\
                  Examples:\n\n\
                  VERDICT: CONFIRMED\n\
                  SUMMARY: zero hub.db references found in tracked source files\n\
                  EVIDENCE: git grep -l 'hub.db' returned no matches\n\n\
                  VERDICT: REFUTED\n\
                  SUMMARY: hex-cli/src/commands/legacy.rs still uses SQLite\n\
                  EVIDENCE: hex-cli/src/commands/legacy.rs:42\n\n\
                  VERDICT: INCONCLUSIVE\n\
                  SUMMARY: claim depends on runtime behavior we cannot inspect statically\n\
                  EVIDENCE: unknown\n\n\
                  Begin your reply with the literal word VERDICT. No preamble.";

    // Pin nemotron-mini: it follows the 3-line format reliably (0.99 on
    // commit-mode in the persona bench), faster than qwen3.5:9b for this
    // kind of structured-output ask. Override with HEX_VERIFY_MODEL.
    let model = std::env::var("HEX_VERIFY_MODEL")
        .unwrap_or_else(|_| "nemotron-mini".to_string());
    let body = serde_json::json!({
        "model": model,
        "system": system,
        "messages": [{"role": "user", "content": format!("Claim: {}", claim)}],
        "max_tokens": 200,
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    let resp = client.post(&inference_url).json(&body).send().await?;
    let v: serde_json::Value = resp.json().await?;
    let content = v.get("content").and_then(|c| c.as_str()).unwrap_or("").to_string();
    let content = Regex::new(r"(?s)<think>.*?</think>")
        .unwrap()
        .replace_all(&content, "")
        .trim()
        .to_string();

    let verdict = Regex::new(r"(?im)^VERDICT:\s*(CONFIRMED|REFUTED|INCONCLUSIVE)").unwrap();
    let summary = Regex::new(r"(?im)^SUMMARY:\s*(.+)$").unwrap();
    let evidence = Regex::new(r"(?im)^EVIDENCE:\s*(.+)$").unwrap();

    let v_str = verdict.captures(&content)
        .and_then(|c| c.get(1)).map(|m| m.as_str().to_uppercase())
        .unwrap_or_else(|| "INCONCLUSIVE".into());
    let s_str = summary.captures(&content)
        .and_then(|c| c.get(1)).map(|m| m.as_str().trim().to_string())
        .unwrap_or_else(|| "model did not produce a SUMMARY line".into());
    let e_str = evidence.captures(&content)
        .and_then(|c| c.get(1)).map(|m| m.as_str().trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    Ok(CheckResult {
        verdict: match v_str.as_str() {
            "CONFIRMED" => Verdict::Confirmed,
            "REFUTED"   => Verdict::Refuted,
            _           => Verdict::Inconclusive,
        },
        summary: s_str,
        evidence: vec![e_str],
        method: "adversarial-llm",
    })
}
