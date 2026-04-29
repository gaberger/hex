//! ADR draft writer (workplan `wp-idle-research-swarm`, P4.2).
//!
//! Routes a [`Finding`] whose `suggested_action.kind == ActionKind::DraftAdr`
//! into a hand-reviewable markdown stub at
//! `docs/adrs/drafts/ADR-<finding-id>.md`. Drafts are *always* written with
//! `Status: Proposed`; this writer NEVER promotes an ADR to `Accepted`. That
//! transition is reserved for a human reviewer (or a separate workflow that
//! gates on review/consensus) so the idle-research swarm cannot quietly
//! enshrine its own opinions as architectural decisions.
//!
//! Idempotency: if the target file already exists the writer returns the
//! existing path unchanged. This protects hand-edits made by reviewers
//! between sweeps — a re-run of the same finding will not clobber an
//! in-progress draft.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use chrono::Utc;
use hex_core::{ActionKind, Domain, Finding, Severity};

/// Errors produced by [`write_adr_draft`].
#[derive(Debug)]
pub enum DraftError {
    /// The finding's `suggested_action.kind` was not `DraftAdr`. Callers
    /// should filter before calling, but we return this rather than silently
    /// no-op'ing so a misrouted finding is loud, not lost.
    WrongKind(ActionKind),
    /// Failed to create the `docs/adrs/drafts/` directory.
    CreateDir {
        path: PathBuf,
        source: io::Error,
    },
    /// Failed to write the draft file.
    Write {
        path: PathBuf,
        source: io::Error,
    },
}

impl std::fmt::Display for DraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DraftError::WrongKind(k) => write!(
                f,
                "draft_writer: expected ActionKind::DraftAdr, got {k:?}"
            ),
            DraftError::CreateDir { path, source } => {
                write!(f, "failed to create {}: {}", path.display(), source)
            }
            DraftError::Write { path, source } => {
                write!(f, "failed to write {}: {}", path.display(), source)
            }
        }
    }
}

impl std::error::Error for DraftError {}

/// Outcome of attempting to write an ADR draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftOutcome {
    /// A new draft was written at this path.
    Written(PathBuf),
    /// A draft for this finding already exists at this path; it was left
    /// untouched.
    Skipped(PathBuf),
}

impl DraftOutcome {
    /// Borrow the path regardless of whether the draft was written or
    /// already existed.
    pub fn path(&self) -> &Path {
        match self {
            DraftOutcome::Written(p) | DraftOutcome::Skipped(p) => p.as_path(),
        }
    }
}

/// Write a `Proposed` ADR draft for `finding` under `project_root`.
///
/// Returns the target path. The destination is
/// `<project_root>/docs/adrs/drafts/ADR-<finding-id>.md`. Existing drafts are
/// left in place (see module-level idempotency note).
///
/// `finding.suggested_action.kind` MUST be [`ActionKind::DraftAdr`]; any
/// other variant is rejected with [`DraftError::WrongKind`].
pub fn write_adr_draft(
    finding: &Finding,
    project_root: &Path,
) -> Result<DraftOutcome, DraftError> {
    if finding.suggested_action.kind != ActionKind::DraftAdr {
        return Err(DraftError::WrongKind(
            finding.suggested_action.kind.clone(),
        ));
    }

    let drafts_dir = project_root.join("docs").join("adrs").join("drafts");
    fs::create_dir_all(&drafts_dir).map_err(|source| DraftError::CreateDir {
        path: drafts_dir.clone(),
        source,
    })?;

    let target = drafts_dir.join(format!("ADR-{}.md", sanitize_id(&finding.id)));
    if target.exists() {
        return Ok(DraftOutcome::Skipped(target));
    }

    let body = render_adr_markdown(finding, &Utc::now().format("%Y-%m-%d").to_string());
    fs::write(&target, body).map_err(|source| DraftError::Write {
        path: target.clone(),
        source,
    })?;

    Ok(DraftOutcome::Written(target))
}

/// Render the markdown body for a draft ADR. Pure (no IO) so it can be unit
/// tested without touching the filesystem.
pub fn render_adr_markdown(finding: &Finding, today: &str) -> String {
    let title = title_from_finding(finding);
    let drivers = format!(
        "Idle-research sweep finding `{}` ({} / {})",
        finding.id,
        domain_label(&finding.domain),
        severity_label(finding.severity),
    );

    let evidence_block = if finding.evidence.is_empty() {
        "_No evidence captured by the analyst._".to_string()
    } else {
        finding
            .evidence
            .iter()
            .map(|e| format!("- `{}`", escape_inline_code(e)))
            .collect::<Vec<_>>()
            .join("\n")
    };

    format!(
        "# ADR-{id}: {title}\n\
         \n\
         **Status:** Proposed\n\
         **Date:** {today}\n\
         **Drivers:** {drivers}\n\
         **Source:** Auto-drafted by `hex-nexus::research::draft_writer` from idle-research finding `{id}`. Drafts are never auto-promoted; a human reviewer must update Status before this becomes an Accepted decision.\n\
         \n\
         ## Context\n\
         \n\
         The idle-research swarm flagged the following while sweeping the project:\n\
         \n\
         > {title}\n\
         \n\
         ### Evidence\n\
         \n\
         {evidence_block}\n\
         \n\
         ## Decision\n\
         \n\
         _Pending human review._ The analyst surfaced the issue above but did not propose a binding decision. Replace this section with the chosen approach, key design details, and scope boundaries before promoting Status to `Accepted`.\n\
         \n\
         ## Consequences\n\
         \n\
         **Positive:**\n\
         - _To be filled in once a decision is recorded._\n\
         \n\
         **Negative:**\n\
         - _To be filled in once a decision is recorded._\n\
         \n\
         **Mitigations:**\n\
         - _To be filled in once a decision is recorded._\n\
         \n\
         ## References\n\
         \n\
         - Workplan: `docs/workplans/wp-idle-research-swarm.json`\n\
         - Finding id: `{id}`\n\
         - Finding domain: `{domain}`\n\
         - Finding severity: `{severity}`\n",
        id = finding.id,
        title = title,
        today = today,
        drivers = drivers,
        evidence_block = evidence_block,
        domain = domain_label(&finding.domain),
        severity = severity_label(finding.severity),
    )
}

fn title_from_finding(finding: &Finding) -> String {
    if finding.title.trim().is_empty() {
        format!("Idle-research finding {}", finding.id)
    } else {
        finding.title.trim().to_string()
    }
}

fn domain_label(domain: &Domain) -> String {
    match domain {
        Domain::Architecture => "architecture".into(),
        Domain::CodeQuality => "code_quality".into(),
        Domain::Drift => "drift".into(),
        Domain::Performance => "performance".into(),
        Domain::Security => "security".into(),
        Domain::Documentation => "documentation".into(),
        Domain::Other(s) => s.clone(),
    }
}

fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Info => "info",
        Severity::Low => "low",
        Severity::Medium => "medium",
        Severity::High => "high",
        Severity::Critical => "critical",
    }
}

/// Restrict the finding id to a filename-safe charset. The analysts already
/// emit ids of the form `f-arch-rust-<hex>` or `f-info-1`, but we defensively
/// strip anything that could escape the drafts directory or break Windows
/// checkouts.
fn sanitize_id(id: &str) -> String {
    let cleaned: String = id
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' => c,
            _ => '-',
        })
        .collect();
    if cleaned.is_empty() {
        "unknown".into()
    } else {
        cleaned
    }
}

fn escape_inline_code(s: &str) -> String {
    s.replace('`', "ʼ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::SuggestedAction;
    use std::env;

    fn adr_finding() -> Finding {
        Finding {
            id: "f-arch-rust-deadbeefcafebabe".into(),
            domain: Domain::Architecture,
            severity: Severity::High,
            title: "adapter→adapter import violates hex rules".into(),
            evidence: vec![
                "adapters/primary/cli.rs:42 imports adapters/secondary/db.rs".into(),
                "hex analyze . --json: violations[0].rule = adr-hex-adapter-isolation".into(),
            ],
            suggested_action: SuggestedAction {
                kind: ActionKind::DraftAdr,
                draft_ref: None,
            },
        }
    }

    fn unique_tmp_root(tag: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let pid = std::process::id();
        let root = env::temp_dir().join(format!("hex-adr-draft-writer-{tag}-{pid}-{nanos}"));
        fs::create_dir_all(&root).expect("create tmp root");
        root
    }

    #[test]
    fn adr_draft_writer_writes_proposed_draft() {
        let root = unique_tmp_root("happy");
        let outcome = write_adr_draft(&adr_finding(), &root).expect("write draft");

        let path = match &outcome {
            DraftOutcome::Written(p) => p.clone(),
            DraftOutcome::Skipped(p) => panic!("expected Written, got Skipped({p:?})"),
        };

        assert!(path.starts_with(root.join("docs").join("adrs").join("drafts")));
        assert_eq!(
            path.file_name().and_then(|s| s.to_str()),
            Some("ADR-f-arch-rust-deadbeefcafebabe.md")
        );

        let body = fs::read_to_string(&path).expect("read draft");
        assert!(body.contains("**Status:** Proposed"), "body = {body}");
        assert!(
            !body.contains("**Status:** Accepted"),
            "drafts must never be marked Accepted; body = {body}"
        );
        assert!(
            body.contains("adapter→adapter import violates hex rules"),
            "body = {body}"
        );
        assert!(
            body.contains("adapters/primary/cli.rs:42 imports adapters/secondary/db.rs"),
            "body = {body}"
        );
        assert!(body.contains("f-arch-rust-deadbeefcafebabe"), "body = {body}");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn adr_draft_writer_is_idempotent() {
        let root = unique_tmp_root("idem");
        let first = write_adr_draft(&adr_finding(), &root).expect("write first");
        let path = first.path().to_path_buf();
        fs::write(&path, "HUMAN EDITS — DO NOT CLOBBER").expect("hand-edit draft");

        let second = write_adr_draft(&adr_finding(), &root).expect("write second");
        match &second {
            DraftOutcome::Skipped(p) => assert_eq!(*p, path),
            DraftOutcome::Written(p) => panic!("second call clobbered hand edits at {p:?}"),
        }

        let body = fs::read_to_string(&path).expect("read draft");
        assert_eq!(body, "HUMAN EDITS — DO NOT CLOBBER");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn adr_draft_writer_rejects_non_adr_findings() {
        let root = unique_tmp_root("wrong-kind");
        let mut f = adr_finding();
        f.suggested_action.kind = ActionKind::DraftWorkplan;
        let err = write_adr_draft(&f, &root).expect_err("should reject");
        assert!(matches!(err, DraftError::WrongKind(ActionKind::DraftWorkplan)));

        let drafts_dir = root.join("docs").join("adrs").join("drafts");
        assert!(
            !drafts_dir.join("ADR-f-arch-rust-deadbeefcafebabe.md").exists(),
            "no file should have been written"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn render_adr_markdown_stable_shape() {
        let body = render_adr_markdown(&adr_finding(), "2026-04-29");
        assert!(body.starts_with("# ADR-f-arch-rust-deadbeefcafebabe: adapter→adapter import violates hex rules"));
        assert!(body.contains("**Date:** 2026-04-29"));
        assert!(body.contains("**Status:** Proposed"));
        assert!(body.contains("Drafts are never auto-promoted"));
        assert!(body.contains("## Context"));
        assert!(body.contains("## Decision"));
        assert!(body.contains("## Consequences"));
        assert!(body.contains("## References"));
    }

    #[test]
    fn render_handles_empty_evidence() {
        let mut f = adr_finding();
        f.evidence.clear();
        let body = render_adr_markdown(&f, "2026-04-29");
        assert!(body.contains("_No evidence captured by the analyst._"));
    }

    #[test]
    fn sanitize_id_strips_path_separators() {
        assert_eq!(sanitize_id("../../etc/passwd"), "------etc-passwd");
        assert_eq!(sanitize_id("f-info-1"), "f-info-1");
        assert_eq!(sanitize_id(""), "unknown");
        assert_eq!(sanitize_id("a/b\\c d"), "a-b-c-d");
    }
}
