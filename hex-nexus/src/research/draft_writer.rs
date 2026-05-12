//! Draft writers for the idle-research swarm
//! (workplan `wp-idle-research-swarm`, P4.2 + P4.3).
//!
//! Each `Finding` carries a `suggested_action.kind`. This module routes those
//! kinds into the matching surface:
//!
//! | `ActionKind`                     | Writer                       | Output                                                              |
//! |----------------------------------|------------------------------|---------------------------------------------------------------------|
//! | `DraftAdr`                       | [`write_adr_draft`]          | `docs/adrs/drafts/ADR-<finding-id>.md` (`Status: Proposed`)         |
//! | `DraftWorkplan` / `AmendWorkplan`| [`write_workplan_draft`]     | `docs/workplans/drafts/draft-<finding-id>.json` (pending-planner)   |
//! | `Memory`                         | [`store_memory_finding`]     | HexFlo memory entry under namespace `idle-sweep`                    |
//! | `Informational`                  | (none — the finding is left in the sweep YAML for humans to read)                                  |
//!
//! ADR drafts are *always* written with `Status: Proposed`; this writer
//! NEVER promotes an ADR to `Accepted`. Workplan drafts are written with
//! `status: "pending-planner"` and are picked up by the existing
//! `hex plan drafts` flow — they are NEVER auto-promoted to executable
//! workplans. Memory entries are stored under a dedicated namespace so a
//! later cleanup pass can scope-delete idle-sweep noise without touching
//! durable swarm memory.
//!
//! Idempotency (file writers only): if the target file already exists the
//! writer returns the existing path unchanged. This protects hand-edits
//! made by reviewers between sweeps — a re-run of the same finding will
//! not clobber an in-progress draft. [`store_memory_finding`] does not
//! diff before writing because the underlying memory store is already
//! upsert-semantics; re-running it simply refreshes the value.

use std::fs;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use hex_core::{ActionKind, Domain, Finding, Severity};
use serde_json::{json, Value};

/// Memory namespace used for findings routed via [`store_memory_finding`].
/// Matches ADR-2026-04-15-1200 §6 ("`kind: memory` → call `hex memory store` with
/// namespace `idle-sweep`"). Centralised so the constant can be reused by
/// any caller scoping a query/list back to idle-sweep entries.
pub const MEMORY_NAMESPACE: &str = "idle-sweep";

/// Errors produced by the writers in this module.
///
/// The writers share an error type so a coordinator can collect a uniform
/// `Vec<DraftError>` regardless of which kind of artifact failed.
#[derive(Debug)]
pub enum DraftError {
    /// The finding's `suggested_action.kind` was not the variant the writer
    /// expects. Callers should filter before invoking, but we return this
    /// rather than silently no-op'ing so a misrouted finding is loud, not
    /// lost.
    WrongKind(ActionKind),
    /// Failed to create the destination directory
    /// (`docs/adrs/drafts/` or `docs/workplans/drafts/`).
    CreateDir {
        path: PathBuf,
        source: io::Error,
    },
    /// Failed to write the draft file.
    Write {
        path: PathBuf,
        source: io::Error,
    },
    /// `serde_json::to_string_pretty` rejected the workplan-draft document.
    /// Should be unreachable — every embedded type is a plain JSON shape —
    /// but we surface it so the caller never sees a panic.
    SerializeWorkplan(serde_json::Error),
    /// The configured memory store returned an error when asked to persist a
    /// memory-routed finding.
    MemoryStore {
        namespace: String,
        key: String,
        source: String,
    },
}

impl std::fmt::Display for DraftError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DraftError::WrongKind(k) => write!(
                f,
                "draft_writer: suggested_action.kind {k:?} is not handled by this writer"
            ),
            DraftError::CreateDir { path, source } => {
                write!(f, "failed to create {}: {}", path.display(), source)
            }
            DraftError::Write { path, source } => {
                write!(f, "failed to write {}: {}", path.display(), source)
            }
            DraftError::SerializeWorkplan(e) => {
                write!(f, "failed to serialize workplan draft as JSON: {e}")
            }
            DraftError::MemoryStore {
                namespace,
                key,
                source,
            } => write!(
                f,
                "memory_store failed for {namespace}/{key}: {source}"
            ),
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

/// Write a `pending-planner` workplan-draft JSON for `finding` under
/// `project_root`.
///
/// The destination is
/// `<project_root>/docs/workplans/drafts/draft-<finding-id>.json`. The
/// document shape matches existing T3 auto-invoke drafts (id / kind /
/// origin / adr / created_at / status / prompt / next_steps / notes) so
/// `hex plan drafts list|approve|clear` accepts it without modification.
/// Idle-research drafts add `finding_id`, `domain`, `severity`, and
/// `evidence` fields so a reviewer can see the analyst's signal without
/// chasing back to the sweep YAML.
///
/// `finding.suggested_action.kind` MUST be [`ActionKind::DraftWorkplan`] or
/// [`ActionKind::AmendWorkplan`]. `AmendWorkplan` is accepted because the
/// existing draft surface is the simplest place a human reviewer can decide
/// whether to spin up a new workplan or splice the finding into an existing
/// one — there is no "amend in place" automation today, and routing the
/// finding into the noop path would silently drop it.
///
/// Idempotent: if the target file already exists the writer returns the
/// existing path unchanged, preserving any hand edits.
pub fn write_workplan_draft(
    finding: &Finding,
    project_root: &Path,
) -> Result<DraftOutcome, DraftError> {
    if !matches!(
        finding.suggested_action.kind,
        ActionKind::DraftWorkplan | ActionKind::AmendWorkplan
    ) {
        return Err(DraftError::WrongKind(
            finding.suggested_action.kind.clone(),
        ));
    }

    let drafts_dir = project_root
        .join("docs")
        .join("workplans")
        .join("drafts");
    fs::create_dir_all(&drafts_dir).map_err(|source| DraftError::CreateDir {
        path: drafts_dir.clone(),
        source,
    })?;

    let draft_id = format!("draft-{}", sanitize_id(&finding.id));
    let target = drafts_dir.join(format!("{draft_id}.json"));
    if target.exists() {
        return Ok(DraftOutcome::Skipped(target));
    }

    let body = render_workplan_draft_json(finding, &Utc::now(), &draft_id)?;
    fs::write(&target, body).map_err(|source| DraftError::Write {
        path: target.clone(),
        source,
    })?;

    Ok(DraftOutcome::Written(target))
}

/// Render the JSON body for a workplan draft. Pure (no IO) so it can be
/// unit-tested without touching the filesystem.
///
/// The shape matches the existing T3 auto-invoke draft format
/// (`docs/workplans/drafts/draft-*.json`) so `hex plan drafts` flows pick
/// these up untouched. `mode` (`new` for `DraftWorkplan`, `amend` for
/// `AmendWorkplan`) is appended as an extra field so the planner can pick
/// the right downstream behavior.
pub fn render_workplan_draft_json(
    finding: &Finding,
    now: &DateTime<Utc>,
    draft_id: &str,
) -> Result<String, DraftError> {
    let mode = match finding.suggested_action.kind {
        ActionKind::AmendWorkplan => "amend",
        _ => "new",
    };

    let prompt = title_from_finding(finding);
    let domain = domain_label(&finding.domain);
    let severity = severity_label(finding.severity);

    let next_steps: Vec<Value> = vec![
        json!(format!(
            "Run /hex-feature-dev to expand this draft into a full workplan, or `hex plan drafts approve {draft_id}.json`"
        )),
        json!(format!(
            "Or discard with `hex plan drafts clear --name {draft_id}`"
        )),
    ];

    let notes = format!(
        "Auto-drafted by `hex-nexus::research::draft_writer` from idle-research finding `{}` ({domain} / {severity}). \
         Drafts are pending-planner — no specs, steps, or tiers have been generated yet. \
         Idle-research drafts are NEVER auto-promoted to executable workplans; the planner agent fills in the body when picked up.",
        finding.id
    );

    let doc = json!({
        "id": draft_id,
        "kind": "workplan-draft",
        "origin": "idle-research-swarm",
        "adr": "ADR-2026-04-15-1200",
        "created_at": now.to_rfc3339(),
        "status": "pending-planner",
        "mode": mode,
        "prompt": prompt,
        "finding_id": finding.id,
        "domain": domain,
        "severity": severity,
        "evidence": finding.evidence,
        "next_steps": next_steps,
        "notes": notes,
    });

    serde_json::to_string_pretty(&doc).map_err(DraftError::SerializeWorkplan)
}

/// A prepared memory entry for an idle-sweep finding — the (namespace, key,
/// value) triple that [`store_memory_finding`] hands to the underlying
/// memory store.
///
/// Returned by [`memory_record_for_finding`] (pure) so callers can inspect
/// what would be stored without performing any IO. The record is also the
/// success value of [`store_memory_finding`] — useful for logging /
/// dashboards that want to display "stored idle-sweep/<id>" without
/// re-deriving the key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryRecord {
    /// Memory store namespace / scope. Always [`MEMORY_NAMESPACE`].
    pub namespace: String,
    /// Stable key for the finding within the namespace.
    /// Format: `idle-sweep/<sanitized-finding-id>`. The namespace prefix is
    /// included in the key as well as in the namespace field so the record
    /// remains globally addressable for stores whose `scope` argument is
    /// only advisory (e.g. the current `hex memory store` CLI which omits
    /// `--scope` and stores into `global`).
    pub key: String,
    /// JSON-serialised finding body. The full [`Finding`] is preserved so
    /// `hex memory get` returns enough context for a reviewer to act on
    /// the entry without re-running the sweep.
    pub value: String,
}

/// Build the memory record for a `Memory`-routed finding. Pure (no IO).
///
/// Exposed publicly so tests and dashboards can inspect what would be
/// stored, and so a future batch-store path can pre-compute records before
/// touching the network.
pub fn memory_record_for_finding(finding: &Finding) -> MemoryRecord {
    let key = format!("{}/{}", MEMORY_NAMESPACE, sanitize_id(&finding.id));
    let value = serde_json::to_string(finding)
        .unwrap_or_else(|_| String::from("{\"error\":\"serialize finding failed\"}"));
    MemoryRecord {
        namespace: MEMORY_NAMESPACE.to_string(),
        key,
        value,
    }
}

/// Persist a `Memory`-routed finding into the swarm's memory store under
/// the [`MEMORY_NAMESPACE`] (`idle-sweep`) namespace.
///
/// `store` is a generic async closure `(key, value, namespace) ->
/// Future<Result<(), String>>` so the writer doesn't depend on
/// `coordination::HexFlo` directly — production callers wire HexFlo's
/// `memory_store`, tests pass a closure that pushes into a shared `Vec`
/// for assertion. The closure receives owned `String`s so the future can
/// outlive the borrowed `finding`.
///
/// Re-running the call with the same finding is safe and intentional:
/// memory_store has upsert semantics (see `coordination::memory`), so a
/// later sweep that produces a refined finding refreshes the entry rather
/// than spawning a duplicate.
///
/// `finding.suggested_action.kind` MUST be [`ActionKind::Memory`].
pub async fn store_memory_finding<F, Fut>(
    finding: &Finding,
    store: F,
) -> Result<MemoryRecord, DraftError>
where
    F: FnOnce(String, String, String) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    if finding.suggested_action.kind != ActionKind::Memory {
        return Err(DraftError::WrongKind(
            finding.suggested_action.kind.clone(),
        ));
    }

    let record = memory_record_for_finding(finding);
    store(
        record.key.clone(),
        record.value.clone(),
        record.namespace.clone(),
    )
    .await
    .map_err(|source| DraftError::MemoryStore {
        namespace: record.namespace.clone(),
        key: record.key.clone(),
        source,
    })?;

    Ok(record)
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
                "hex analyze . --json: violations[0].rule = ADR-hex-adapter-isolation".into(),
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
        let root = env::temp_dir().join(format!("hex-ADR-draft-writer-{tag}-{pid}-{nanos}"));
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

    // ── P4.3: workplan-draft writer ───────────────────────

    fn workplan_finding() -> Finding {
        Finding {
            id: "f-2604241200-arch-cross-adapter".into(),
            domain: Domain::Architecture,
            severity: Severity::High,
            title: "adapter→adapter import violates hex rules".into(),
            evidence: vec![
                "adapters/primary/cli.rs:42 imports adapters/secondary/db.rs".into(),
            ],
            suggested_action: SuggestedAction {
                kind: ActionKind::DraftWorkplan,
                draft_ref: None,
            },
        }
    }

    #[test]
    fn workplan_draft_writer_writes_pending_planner_json() {
        let root = unique_tmp_root("wp-happy");
        let outcome = write_workplan_draft(&workplan_finding(), &root).expect("write");

        let path = match &outcome {
            DraftOutcome::Written(p) => p.clone(),
            DraftOutcome::Skipped(p) => panic!("expected Written, got Skipped({p:?})"),
        };

        assert!(
            path.starts_with(root.join("docs").join("workplans").join("drafts")),
            "path = {path:?}"
        );
        assert_eq!(
            path.file_name().and_then(|s| s.to_str()),
            Some("draft-f-2604241200-arch-cross-adapter.json")
        );

        let body = fs::read_to_string(&path).expect("read draft");
        let parsed: serde_json::Value =
            serde_json::from_str(&body).expect("workplan draft must be valid JSON");

        assert_eq!(parsed["kind"], "workplan-draft");
        assert_eq!(parsed["status"], "pending-planner");
        assert_eq!(parsed["origin"], "idle-research-swarm");
        assert_eq!(parsed["adr"], "ADR-2026-04-15-1200");
        assert_eq!(parsed["mode"], "new");
        assert_eq!(parsed["finding_id"], "f-2604241200-arch-cross-adapter");
        assert_eq!(parsed["domain"], "architecture");
        assert_eq!(parsed["severity"], "high");
        assert_eq!(parsed["id"], "draft-f-2604241200-arch-cross-adapter");
        assert!(
            parsed["evidence"]
                .as_array()
                .map(|a| !a.is_empty())
                .unwrap_or(false),
            "evidence array should round-trip"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workplan_draft_writer_marks_amend_in_mode_field() {
        let root = unique_tmp_root("wp-amend");
        let mut f = workplan_finding();
        f.suggested_action.kind = ActionKind::AmendWorkplan;

        let outcome = write_workplan_draft(&f, &root).expect("write");
        let body = fs::read_to_string(outcome.path()).expect("read draft");
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");

        assert_eq!(parsed["mode"], "amend");
        assert_eq!(parsed["status"], "pending-planner");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workplan_draft_writer_is_idempotent() {
        let root = unique_tmp_root("wp-idem");
        let first = write_workplan_draft(&workplan_finding(), &root).expect("first");
        let path = first.path().to_path_buf();
        fs::write(&path, "HUMAN EDITS — DO NOT CLOBBER").expect("hand-edit");

        let second = write_workplan_draft(&workplan_finding(), &root).expect("second");
        match &second {
            DraftOutcome::Skipped(p) => assert_eq!(*p, path),
            DraftOutcome::Written(p) => panic!("clobbered at {p:?}"),
        }

        let body = fs::read_to_string(&path).expect("read draft");
        assert_eq!(body, "HUMAN EDITS — DO NOT CLOBBER");

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workplan_draft_writer_rejects_non_workplan_findings() {
        let root = unique_tmp_root("wp-wrong-kind");
        let mut f = workplan_finding();
        f.suggested_action.kind = ActionKind::DraftAdr;

        let err = write_workplan_draft(&f, &root).expect_err("should reject");
        assert!(matches!(err, DraftError::WrongKind(ActionKind::DraftAdr)));

        let drafts_dir = root.join("docs").join("workplans").join("drafts");
        assert!(
            !drafts_dir.exists()
                || drafts_dir.read_dir().map(|mut d| d.next().is_none()).unwrap_or(true),
            "no file should have been written"
        );

        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn workplan_draft_writer_rejects_memory_findings() {
        // Memory findings must NOT be silently routed into the workplan
        // draft surface; they have a dedicated store path.
        let root = unique_tmp_root("wp-mem");
        let mut f = workplan_finding();
        f.suggested_action.kind = ActionKind::Memory;
        let err = write_workplan_draft(&f, &root).expect_err("should reject");
        assert!(matches!(err, DraftError::WrongKind(ActionKind::Memory)));
        fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn render_workplan_draft_json_has_stable_shape() {
        use chrono::TimeZone;
        let now = Utc.with_ymd_and_hms(2026, 4, 29, 12, 34, 56).unwrap();
        let body =
            render_workplan_draft_json(&workplan_finding(), &now, "draft-test").expect("render");

        let parsed: serde_json::Value = serde_json::from_str(&body).expect("valid JSON");
        assert_eq!(parsed["id"], "draft-test");
        assert_eq!(parsed["created_at"], "2026-04-29T12:34:56+00:00");
        assert_eq!(parsed["mode"], "new");
        assert!(parsed["next_steps"].is_array());
        assert!(parsed["notes"].as_str().unwrap().contains("Idle-research"));
    }

    // ── P4.3: memory-store routing ────────────────────────

    fn memory_finding() -> Finding {
        Finding {
            id: "f-info-1".into(),
            domain: Domain::Documentation,
            severity: Severity::Info,
            title: "README mentions removed flag --foo".into(),
            evidence: vec!["README.md:120".into()],
            suggested_action: SuggestedAction {
                kind: ActionKind::Memory,
                draft_ref: None,
            },
        }
    }

    #[test]
    fn memory_record_for_finding_uses_idle_sweep_namespace() {
        let record = memory_record_for_finding(&memory_finding());
        assert_eq!(record.namespace, MEMORY_NAMESPACE);
        assert_eq!(record.namespace, "idle-sweep");
        assert_eq!(record.key, "idle-sweep/f-info-1");

        // Value round-trips back to a Finding.
        let back: Finding = serde_json::from_str(&record.value).expect("round-trip");
        assert_eq!(back.id, "f-info-1");
        assert_eq!(back.suggested_action.kind, ActionKind::Memory);
    }

    #[test]
    fn memory_record_sanitizes_id_in_key() {
        let mut f = memory_finding();
        f.id = "../../etc/passwd".into();
        let record = memory_record_for_finding(&f);
        assert_eq!(record.key, "idle-sweep/------etc-passwd");
    }

    #[tokio::test]
    async fn store_memory_finding_invokes_callback_with_namespace() {
        use std::sync::{Arc, Mutex};

        let calls: Arc<Mutex<Vec<(String, String, String)>>> =
            Arc::new(Mutex::new(Vec::new()));
        let calls_clone = Arc::clone(&calls);

        let store = move |key: String, value: String, namespace: String| {
            let calls = Arc::clone(&calls_clone);
            async move {
                calls.lock().unwrap().push((key, value, namespace));
                Ok::<(), String>(())
            }
        };

        let record = store_memory_finding(&memory_finding(), store)
            .await
            .expect("store ok");

        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 1, "exactly one store call expected");
        let (k, v, ns) = &recorded[0];
        assert_eq!(k, "idle-sweep/f-info-1");
        assert_eq!(ns, "idle-sweep");
        // Stored value must be the JSON-serialised finding.
        let parsed: Finding = serde_json::from_str(v).expect("stored value is JSON Finding");
        assert_eq!(parsed.id, "f-info-1");

        assert_eq!(record.key, "idle-sweep/f-info-1");
        assert_eq!(record.namespace, "idle-sweep");
    }

    #[tokio::test]
    async fn store_memory_finding_propagates_store_errors() {
        let store = |_key: String, _value: String, _namespace: String| async move {
            Err::<(), String>("connection refused".to_string())
        };

        let err = store_memory_finding(&memory_finding(), store)
            .await
            .expect_err("should fail");

        match err {
            DraftError::MemoryStore {
                namespace,
                key,
                source,
            } => {
                assert_eq!(namespace, "idle-sweep");
                assert_eq!(key, "idle-sweep/f-info-1");
                assert_eq!(source, "connection refused");
            }
            other => panic!("unexpected error variant: {other:?}"),
        }
    }

    #[tokio::test]
    async fn store_memory_finding_rejects_non_memory_findings() {
        let mut f = memory_finding();
        f.suggested_action.kind = ActionKind::DraftAdr;

        // Closure must NOT be invoked for misrouted findings.
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;
        let invoked = Arc::new(AtomicBool::new(false));
        let invoked_clone = Arc::clone(&invoked);

        let store = move |_k: String, _v: String, _ns: String| {
            let invoked = Arc::clone(&invoked_clone);
            async move {
                invoked.store(true, Ordering::SeqCst);
                Ok::<(), String>(())
            }
        };

        let err = store_memory_finding(&f, store).await.expect_err("should reject");
        assert!(matches!(err, DraftError::WrongKind(ActionKind::DraftAdr)));
        assert!(
            !invoked.load(Ordering::SeqCst),
            "store callback must not run for misrouted findings"
        );
    }
}
