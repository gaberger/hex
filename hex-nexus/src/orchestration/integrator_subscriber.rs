//! Integrator subscriber (ADR-2026-05-08-1126 P4).
//!
//! Drives the merge-team gate end-to-end:
//!
//!   1. Polls `merge_request WHERE status='pending'` → transitions to
//!      `voting` and dispatches the validation-judge.
//!
//!   2. Polls `merge_request WHERE status='voting'` → calls
//!      `merge_decision_tally` (the reducer flips status based on votes
//!      vs the per-pool quorum policy).
//!
//!   3. Polls `merge_request WHERE status='approved'` → runs `hex worktree
//!      merge <branch>` and on success transitions to `merged`.
//!
//!   4. Polls `merge_request WHERE status='rejected'` → posts an inbox
//!      notification (best-effort) so the operator sees stalled merges.
//!
//! ## Voter dispatch
//!
//! Today only the validation-judge is automated. Its vote is deterministic:
//! `cargo check --workspace` inside the worktree → `pass` if exit 0,
//! `fail` otherwise (with the cargo output trimmed into the reason field,
//! capped at 4 KB to fit MergeVote.reason).
//!
//! Adversarial-red and adversarial-blue voters are NOT dispatched yet;
//! they require an LLM-driven agent that reads diffs and votes on
//! correctness/security. Until they ship, the per-pool quorum policy
//! must be configured to clear with judge=pass alone (`min_pass_votes=1,
//! require_judge_pass=true`) — the default policy of 2-of-3 will leave
//! merges stuck in `voting`. Document this in the operator runbook when
//! P5 ships.
//!
//! Disable via `HEX_DISABLE_INTEGRATOR_SUBSCRIBER=1` (mirrors the
//! org_responder pattern).

use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::time;
use tracing::{debug, info, warn};

const POLL_INTERVAL_SECS: u64 = 5;
const STARTUP_DELAY_SECS: u64 = 10;
const REASON_MAX_BYTES: usize = 4000; // matches MergeVote.reason cap (4 KB).
/// Cap on the cargo-check duration so a bad worktree doesn't pin the
/// subscriber.
const JUDGE_TIMEOUT_SECS: u64 = 180;

#[derive(Clone)]
pub struct IntegratorSubscriber {
    http: reqwest::Client,
    stdb_host: String,
    hex_db: String,
}

impl IntegratorSubscriber {
    pub fn new(stdb_host: String, hex_db: String) -> Self {
        // Timeout has to cover the slow path: an LLM adversarial review
        // can take 30-60s on a cold provider. STDB calls finish in
        // milliseconds, so a single long timeout is fine.
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(120))
            .pool_max_idle_per_host(4)
            .build()
            .expect("integrator http client");
        Self { http, stdb_host, hex_db }
    }
}

pub fn spawn(stdb_host: String, hex_db: String) {
    if std::env::var("HEX_DISABLE_INTEGRATOR_SUBSCRIBER").is_ok() {
        info!("integrator_subscriber disabled via HEX_DISABLE_INTEGRATOR_SUBSCRIBER");
        return;
    }
    let inst = Arc::new(IntegratorSubscriber::new(stdb_host, hex_db));
    tokio::spawn(async move {
        // Give STDB time to settle after nexus startup republish.
        time::sleep(Duration::from_secs(STARTUP_DELAY_SECS)).await;
        info!("integrator_subscriber: started");
        let mut ticker = time::interval(Duration::from_secs(POLL_INTERVAL_SECS));
        ticker.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(e) = inst.tick().await {
                debug!(error = %e, "integrator_subscriber tick error");
            }
        }
    });
}

impl IntegratorSubscriber {
    async fn tick(&self) -> Result<(), String> {
        // 1. Pending → kick off voting + dispatch all three voters in parallel.
        for mr in self.list_by_status("pending").await? {
            let path = mr.worktree_path.clone();
            info!(worktree = %path, "merge_request: pending → voting");
            if let Err(e) = self.set_status(&path, "voting").await {
                warn!(worktree = %path, error = %e, "transition pending→voting failed");
                continue;
            }
            // Three voters run in parallel:
            //   - validation-judge: deterministic cargo check
            //   - adversarial-red:  LLM on Anthropic provider (security/boundary skeptic)
            //   - adversarial-blue: LLM on OpenAI/local provider (correctness skeptic)
            // Provider divergence catches single-vendor failure modes
            // (e.g. one model getting fooled, or a model-id outage).
            let inst = self.clone();
            let mr_judge = mr.clone();
            tokio::spawn(async move {
                inst.dispatch_judge(&mr_judge).await;
            });
            let inst = self.clone();
            let mr_red = mr.clone();
            tokio::spawn(async move {
                inst.dispatch_adversarial(&mr_red, "adversarial-red", "anthropic").await;
            });
            let inst = self.clone();
            let mr_blue = mr.clone();
            tokio::spawn(async move {
                inst.dispatch_adversarial(&mr_blue, "adversarial-blue", "openai").await;
            });
        }

        // 2. Voting → tally (the reducer transitions if quorum reached).
        for mr in self.list_by_status("voting").await? {
            let _ = self.call_reducer("merge_decision_tally", &[mr.worktree_path.clone()]).await;
        }

        // 3. Approved → run hex worktree merge, transition to merged.
        for mr in self.list_by_status("approved").await? {
            let path = mr.worktree_path.clone();
            let branch = mr.branch.clone();
            info!(worktree = %path, branch = %branch, "merge_request: approved → merging");
            match run_worktree_merge(&branch).await {
                Ok(()) => {
                    if let Err(e) = self.set_status(&path, "merged").await {
                        warn!(worktree = %path, error = %e, "transition approved→merged failed");
                    } else {
                        info!(worktree = %path, "merge_request: merged");
                    }
                }
                Err(e) => {
                    warn!(worktree = %path, error = %e, "hex worktree merge failed; staying approved for retry");
                    // Leave status='approved' so the next tick retries.
                }
            }
        }

        // 4. Rejected → operator notification (best-effort, stub for now).
        for mr in self.list_by_status("rejected").await? {
            // Mark a derived flag so we don't notify on every poll.
            // For MVP we just log; an inbox notify reducer will land in P5.
            debug!(worktree = %mr.worktree_path, role = %mr.role, "merge_request rejected (operator action required)");
        }

        Ok(())
    }

    /// Dispatch an adversarial voter. Reads the worktree's git diff vs main,
    /// sends it to inference with a focused skeptic prompt, parses one of
    /// {PASS, FAIL, ABSTAIN}, casts the vote.
    ///
    /// `voter`        — "adversarial-red" | "adversarial-blue"
    /// `provider_pref`— routing hint passed to /api/inference/complete via
    ///                  the `provider` field. The inference router uses
    ///                  this to bias toward one vendor; if the preferred
    ///                  vendor is unavailable, it falls back to free.
    ///
    /// Best-effort: failures abstain (vote=abstain) so the request can
    /// still hit quorum from the other voters.
    async fn dispatch_adversarial(
        &self,
        mr: &MergeRequestRow,
        voter: &str,
        provider_pref: &str,
    ) {
        let path = &mr.worktree_path;
        info!(worktree = %path, voter, provider = provider_pref, "adversarial: reading diff");

        // 1. Get the diff. If git diff fails (e.g. no commits in worktree),
        //    abstain rather than fail-the-merge.
        let diff = match read_worktree_diff(path).await {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!(worktree = %path, voter, error = %e, "diff failed; abstaining");
                let _ = self
                    .call_reducer(
                        "merge_vote_cast",
                        &[
                            path.clone(),
                            voter.to_string(),
                            "abstain".to_string(),
                            format!("could not read diff: {}", e),
                        ],
                    )
                    .await;
                return;
            }
        };

        // 2. Run inference with role-specific skeptic prompt. Transient
        //    self-loopback errors (cargo check saturating the runtime,
        //    inference-gateway briefly stalled, etc.) get one retry after
        //    a short backoff before we abstain. The retry budget is
        //    deliberately tight — we'd rather abstain quickly than block
        //    the merge gate's tally loop.
        let mut attempt = 0u32;
        let mut last_err: String;
        let (verdict, reason) = loop {
            attempt += 1;
            match adversarial_review(
                voter,
                provider_pref,
                &diff,
                self.local_inference_url(),
                &self.http,
            )
            .await
            {
                Ok(r) => break r,
                Err(e) => {
                    last_err = e;
                    if attempt >= 2 {
                        tracing::debug!(
                            worktree = %path, voter, attempt,
                            error = %last_err,
                            "inference failed after retry; abstaining"
                        );
                        break (
                            "abstain".to_string(),
                            format!("inference error after {} attempts: {}", attempt, last_err),
                        );
                    }
                    tracing::debug!(
                        worktree = %path, voter, attempt,
                        error = %last_err,
                        "inference failed; retrying after 5s backoff"
                    );
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        };

        info!(worktree = %path, voter, verdict = %verdict, "adversarial voted");
        let reason_trimmed = if reason.len() > REASON_MAX_BYTES {
            let mut end = REASON_MAX_BYTES;
            while end > 0 && !reason.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…[truncated]", &reason[..end])
        } else {
            reason
        };
        if let Err(e) = self
            .call_reducer(
                "merge_vote_cast",
                &[
                    path.clone(),
                    voter.to_string(),
                    verdict,
                    reason_trimmed,
                ],
            )
            .await
        {
            warn!(worktree = %path, voter, error = %e, "merge_vote_cast (adversarial) failed");
        }
    }

    fn local_inference_url(&self) -> String {
        // Self-loopback to nexus's own /api/inference/complete (matches the
        // org_responder pattern). Avoids hitting STDB inference-gateway
        // directly so the adversarial voters get the same key-mgmt/normalization
        // benefits as the persona responder.
        let port = std::env::var("HEX_NEXUS_PORT").unwrap_or_else(|_| "5555".to_string());
        format!("http://127.0.0.1:{}/api/inference/complete", port)
    }

    async fn dispatch_judge(&self, mr: &MergeRequestRow) {
        let path = &mr.worktree_path;
        info!(worktree = %path, "validation-judge: cargo check --workspace");
        let (passed, reason) = run_cargo_check(path).await;
        let verdict = if passed { "pass" } else { "fail" };
        let reason_trimmed = if reason.len() > REASON_MAX_BYTES {
            // char-boundary safe truncate
            let mut end = REASON_MAX_BYTES;
            while end > 0 && !reason.is_char_boundary(end) {
                end -= 1;
            }
            format!("{}…[truncated]", &reason[..end])
        } else {
            reason
        };
        info!(worktree = %path, verdict, "validation-judge voted");
        if let Err(e) = self
            .call_reducer(
                "merge_vote_cast",
                &[
                    path.clone(),
                    "validation-judge".to_string(),
                    verdict.to_string(),
                    reason_trimmed,
                ],
            )
            .await
        {
            warn!(worktree = %path, error = %e, "merge_vote_cast (judge) failed");
        }
    }

    async fn list_by_status(&self, status: &str) -> Result<Vec<MergeRequestRow>, String> {
        let safe = status.replace('\'', "''");
        let q = format!(
            "SELECT worktree_path, branch, role, status FROM merge_request WHERE status = '{}'",
            safe
        );
        let url = format!("{}/v1/database/{}/sql", self.stdb_host, self.hex_db);
        let resp = self
            .http
            .post(&url)
            .header("Content-Type", "text/plain")
            .body(q)
            .send()
            .await
            .map_err(|e| format!("http: {}", e))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status()));
        }
        let body: Value = resp.json().await.map_err(|e| format!("json: {}", e))?;
        let rows = body
            .as_array()
            .and_then(|a| a.first())
            .and_then(|t| t.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default();
        let mut out = Vec::with_capacity(rows.len());
        for row in rows {
            let cols = match row.as_array() {
                Some(a) if a.len() >= 4 => a,
                _ => continue,
            };
            let worktree_path = cols[0].as_str().unwrap_or("").to_string();
            let branch = cols[1].as_str().unwrap_or("").to_string();
            let role = cols[2].as_str().unwrap_or("").to_string();
            let status_v = cols[3].as_str().unwrap_or("").to_string();
            if !worktree_path.is_empty() {
                out.push(MergeRequestRow {
                    worktree_path,
                    branch,
                    role,
                    _status: status_v,
                });
            }
        }
        Ok(out)
    }

    async fn set_status(&self, worktree_path: &str, new_status: &str) -> Result<(), String> {
        self.call_reducer(
            "merge_request_set_status",
            &[worktree_path.to_string(), new_status.to_string()],
        )
        .await
    }

    async fn call_reducer(&self, reducer: &str, args: &[String]) -> Result<(), String> {
        let url = format!(
            "{}/v1/database/{}/call/{}",
            self.stdb_host, self.hex_db, reducer
        );
        let resp = self
            .http
            .post(&url)
            .json(args)
            .send()
            .await
            .map_err(|e| format!("http: {}", e))?;
        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("reducer {} HTTP {}: {}", reducer, body, body));
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct MergeRequestRow {
    worktree_path: String,
    branch: String,
    role: String,
    _status: String,
}

/// Run `cargo check --workspace` inside the given worktree. Returns
/// (passed, output). Output is the combined stderr+stdout of cargo,
/// useful for the judge's reason field on failure.
///
/// Resolves `cargo` via:
///   1. CARGO env var (set by rustup-managed shells).
///   2. `$HOME/.cargo/bin/cargo` (default rustup install location).
///   3. PATH lookup (last resort; nexus daemon may have minimal PATH).
async fn run_cargo_check(worktree_path: &str) -> (bool, String) {
    let path = std::path::Path::new(worktree_path);
    if !path.exists() {
        return (false, format!("worktree path does not exist: {}", worktree_path));
    }
    let cargo_bin = resolve_cargo();
    // HEX_JUDGE_CARGO_ARGS lets operators scope the judge. Default behavior:
    //   - GTK system deps present (pkg-config sees gtk+-3.0 or libsoup) →
    //     `check --workspace` (broadest coverage)
    //   - GTK absent → scoped to the GTK-free crates (hex-desktop excluded)
    // This keeps the gate working out-of-the-box on minimal Linux installs
    // while giving full coverage on dev boxes that have the system libs.
    let args_env = std::env::var("HEX_JUDGE_CARGO_ARGS").unwrap_or_else(|_| default_judge_cargo_args());
    let args: Vec<&str> = args_env.split_whitespace().collect();
    let mut builder = tokio::process::Command::new(&cargo_bin);
    builder.args(&args).current_dir(path).env("HEX_HUB_BUILD_HASH", "merge-gate-judge");
    let cmd = builder.output();
    let out = match time::timeout(Duration::from_secs(JUDGE_TIMEOUT_SECS), cmd).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return (false, format!("spawn cargo ({}): {}", cargo_bin, e)),
        Err(_) => return (false, format!("cargo check timed out after {}s", JUDGE_TIMEOUT_SECS)),
    };
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), combined)
}

/// Read the worktree's diff vs main. Falls back to `git diff HEAD~1` if
/// `main` isn't a known ref locally. Capped at 32 KB so an enormous diff
/// doesn't push the LLM past its context window.
async fn read_worktree_diff(worktree_path: &str) -> Result<String, String> {
    let path = std::path::Path::new(worktree_path);
    if !path.exists() {
        return Err(format!("worktree path does not exist: {}", worktree_path));
    }
    // Try main..HEAD first; if main isn't local, fall back to HEAD~1..HEAD.
    let try_diff = |refspec: &str| {
        let mut c = tokio::process::Command::new("git");
        c.args(["diff", refspec, "--unified=3"])
            .current_dir(path);
        c
    };
    let primary = match try_diff("main..HEAD").output().await {
        Ok(o) if o.status.success() => Some(o),
        _ => None,
    };
    let out = match primary {
        Some(o) => o,
        None => match try_diff("HEAD~1..HEAD").output().await {
            Ok(o) if o.status.success() => o,
            Ok(o) => return Err(format!(
                "git diff: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            )),
            Err(e) => return Err(format!("spawn git diff: {}", e)),
        },
    };
    let mut diff = String::from_utf8_lossy(&out.stdout).to_string();
    const MAX_DIFF_BYTES: usize = 32 * 1024;
    if diff.len() > MAX_DIFF_BYTES {
        let mut end = MAX_DIFF_BYTES;
        while end > 0 && !diff.is_char_boundary(end) {
            end -= 1;
        }
        diff.truncate(end);
        diff.push_str("\n…[diff truncated to 32 KB]");
    }
    if diff.trim().is_empty() {
        return Err("empty diff".into());
    }
    Ok(diff)
}

/// LLM-driven adversarial review. Sends the diff to the inference endpoint
/// with a role-specific skeptic system prompt, expects a structured one-liner
/// reply: `VERDICT: <pass|fail|abstain> — <one-line reason>`.
///
/// Returns (verdict, reason). On parse failure → abstain with the raw
/// content as reason (still useful for audit).
async fn adversarial_review(
    role: &str,
    provider_pref: &str,
    diff: &str,
    inference_url: String,
    http: &reqwest::Client,
) -> Result<(String, String), String> {
    let system = match role {
        "adversarial-red" => {
            "You are adversarial-red — a security/boundary skeptic reviewing a code diff. \
             Look for: ADR violations, hexagonal architecture boundary breaks (e.g. domain \
             importing adapters, ports importing adapters), secret/credential leaks, unsafe \
             input handling, missing path traversal checks, dependency tampering. \
             Respond with EXACTLY ONE LINE in this format:\n\
             VERDICT: <pass|fail|abstain> — <reason in 30 words max>\n\
             Use 'pass' if the diff is safe. Use 'fail' if you found a clear violation. \
             Use 'abstain' only if the diff is too vague to judge."
        }
        "adversarial-blue" => {
            "You are adversarial-blue — a correctness/UX skeptic reviewing a code diff. \
             Look for: test-mirror-bugs (same author wrote tests + code), error-message lies, \
             sign-convention reversals, spec drift, missing edge cases, off-by-one errors, \
             silent fallbacks that hide failures. \
             Respond with EXACTLY ONE LINE in this format:\n\
             VERDICT: <pass|fail|abstain> — <reason in 30 words max>\n\
             Use 'pass' if the diff looks correct. Use 'fail' if you found a clear bug. \
             Use 'abstain' only if you can't reach a conclusion."
        }
        _ => "You are an adversarial reviewer. Reply with VERDICT: <pass|fail|abstain> — <reason>.",
    };
    let body = serde_json::json!({
        "messages": [{
            "role": "user",
            "content": format!(
                "Review this diff. Respond on ONE line in the prescribed format.\n\n```diff\n{}\n```",
                diff
            ),
        }],
        "system": system,
        "max_tokens": 96,
        "provider": provider_pref,
    });
    let resp = http
        .post(&inference_url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("http: {}", e))?;
    let status = resp.status();
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("json: {}", e))?;
    if !status.is_success() {
        return Err(format!("HTTP {}: {}", status, json));
    }
    let content = json
        .get("content")
        .and_then(|c| c.as_str())
        .ok_or_else(|| format!("missing content: {}", json))?
        .trim()
        .to_string();

    Ok(parse_adversarial_verdict(&content))
}

/// Parse `VERDICT: <pass|fail|abstain> — <reason>` (with flexible whitespace
/// and dash variants). Falls back to abstain + raw content if no match.
fn parse_adversarial_verdict(content: &str) -> (String, String) {
    let lower = content.to_lowercase();
    let verdict = if lower.contains("verdict: fail") || lower.starts_with("fail") {
        "fail"
    } else if lower.contains("verdict: pass") || lower.starts_with("pass") {
        "pass"
    } else if lower.contains("verdict: abstain") || lower.contains("abstain") {
        "abstain"
    } else {
        // No verdict marker → abstain so we don't mis-vote.
        "abstain"
    };
    // Reason: strip the "VERDICT: x — " prefix if present.
    let reason = content
        .splitn(2, '—')
        .nth(1)
        .or_else(|| content.splitn(2, '-').nth(1))
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| content.trim().to_string());
    let trimmed = if reason.len() > 800 {
        let mut end = 800;
        while end > 0 && !reason.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &reason[..end])
    } else {
        reason
    };
    (verdict.to_string(), trimmed)
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn parses_pass_verdict() {
        let (v, r) = parse_adversarial_verdict("VERDICT: pass — looks clean to me");
        assert_eq!(v, "pass");
        assert!(r.contains("looks clean"));
    }

    #[test]
    fn parses_fail_verdict() {
        let (v, _) = parse_adversarial_verdict("VERDICT: fail — secret leaked in env");
        assert_eq!(v, "fail");
    }

    #[test]
    fn parses_abstain_verdict() {
        let (v, _) = parse_adversarial_verdict("VERDICT: abstain — diff too vague");
        assert_eq!(v, "abstain");
    }

    #[test]
    fn no_verdict_marker_abstains() {
        let (v, _) = parse_adversarial_verdict("This looks fine I think");
        assert_eq!(v, "abstain");
    }

    #[test]
    fn handles_dash_variant() {
        let (v, r) = parse_adversarial_verdict("VERDICT: pass - no boundary violations seen");
        assert_eq!(v, "pass");
        assert!(r.contains("no boundary violations"));
    }
}

/// Decide the default cargo args for the judge based on whether GTK system
/// libraries are available. Probes via `pkg-config --exists gtk+-3.0`. The
/// result is cached in a `OnceLock` so we only fork pkg-config once per
/// process. Override at any time via the `HEX_JUDGE_CARGO_ARGS` env var.
fn default_judge_cargo_args() -> String {
    use std::sync::OnceLock;
    static CACHED: OnceLock<String> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let gtk_present = std::process::Command::new("pkg-config")
                .args(["--exists", "gtk+-3.0"])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            let args = if gtk_present {
                "check --workspace".to_string()
            } else {
                "check -p hex-cli -p hex-core -p hex-agent -p hex-nexus -p hex-parser -p hex-analyzer"
                    .to_string()
            };
            tracing::info!(
                gtk_present,
                args = %args,
                "judge: default cargo args resolved (override via HEX_JUDGE_CARGO_ARGS)"
            );
            args
        })
        .clone()
}

fn resolve_cargo() -> String {
    if let Ok(env_cargo) = std::env::var("CARGO") {
        if std::path::Path::new(&env_cargo).exists() {
            return env_cargo;
        }
    }
    if let Ok(home) = std::env::var("HOME") {
        let candidate = format!("{}/.cargo/bin/cargo", home);
        if std::path::Path::new(&candidate).exists() {
            return candidate;
        }
    }
    "cargo".to_string()
}

/// Run `hex worktree merge <branch>`. Returns Ok on exit 0; Err with
/// stderr otherwise. The CLI cherry-picks file-level changes per
/// ADR-2026-04-13-1930 — never raw `git checkout`.
async fn run_worktree_merge(branch: &str) -> Result<(), String> {
    let cmd = tokio::process::Command::new("hex")
        .arg("worktree")
        .arg("merge")
        .arg(branch)
        .output();
    let out = match time::timeout(Duration::from_secs(JUDGE_TIMEOUT_SECS), cmd).await {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return Err(format!("spawn hex worktree merge: {}", e)),
        Err(_) => return Err("hex worktree merge timed out".into()),
    };
    if !out.status.success() {
        return Err(format!(
            "hex worktree merge {}: {}",
            branch,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

