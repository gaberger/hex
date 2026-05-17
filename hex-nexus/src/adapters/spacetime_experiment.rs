//! SpacetimeDB-backed adapter implementing [`IExperimentPort`]
//! (ADR-2605021400 P3, wp-experiment-loop-p3b).
//!
//! Tables and reducers live in the `hexflo-coordination` WASM module
//! (database "hex"). Domain enums are translated to/from string columns;
//! reducer error strings are parsed back into [`ExperimentError`] variants.
//!
//! The reference oracle is [`super::in_memory_experiment::InMemoryExperimentAdapter`].
//! The integration test in `hex-nexus/tests/spacetime_experiment_integration.rs`
//! diffs the two against a live SpacetimeDB instance (env-gated).

use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;

use hex_core::domain::experiment::{
    ComparisonOperator, ExperimentError, Hypothesis, HypothesisId, HypothesisStatus, Objective,
    ObjectiveId, ObjectivePriority, ObjectiveStatus, Verdict, VerdictDecision, VerdictId,
};
use hex_core::ports::experiment::IExperimentPort;

/// SpacetimeDB-bound `IExperimentPort` impl.
pub struct SpacetimeExperimentAdapter {
    host: String,
    database: String,
    http: reqwest::Client,
}

impl SpacetimeExperimentAdapter {
    pub fn new(host: String, database: String) -> Self {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .pool_max_idle_per_host(4)
            .build()
            .expect("failed to build HTTP client");
        Self { host, database, http }
    }

    /// Probe connectivity — true if the experiment tables are reachable.
    pub async fn probe(&self) -> bool {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);
        self.http
            .post(&url)
            .body("SELECT id FROM objective LIMIT 1")
            .header("Content-Type", "text/plain")
            .timeout(Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    async fn call_reducer(&self, name: &str, args: Value) -> Result<(), ExperimentError> {
        let url = format!("{}/v1/database/{}/call/{}", self.host, self.database, name);
        let response = self
            .http
            .post(&url)
            .json(&args)
            .send()
            .await
            .map_err(|e| ExperimentError::Backend(format!("SpacetimeDB {name}: {e}")))?;

        if response.status().is_success() {
            return Ok(());
        }

        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        Err(parse_reducer_error(name, status.as_u16(), &body))
    }

    async fn sql_query(&self, query: &str) -> Result<Vec<Value>, ExperimentError> {
        let url = format!("{}/v1/database/{}/sql", self.host, self.database);
        let response = self
            .http
            .post(&url)
            .body(query.to_string())
            .header("Content-Type", "text/plain")
            .send()
            .await
            .map_err(|e| ExperimentError::Backend(format!("SQL query failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(ExperimentError::Backend(format!(
                "SQL query failed ({status}): {body}"
            )));
        }

        let body = response.text().await.unwrap_or_default();
        let parsed: Value = serde_json::from_str(&body)
            .map_err(|e| ExperimentError::Backend(format!("parse SQL response: {e}")))?;

        Ok(parsed
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|table| table.get("rows"))
            .and_then(|r| r.as_array())
            .cloned()
            .unwrap_or_default())
    }

    fn escape_sql(s: &str) -> String {
        s.replace('\'', "''")
    }
}

// ── Error parsing ─────────────────────────────────────────────

fn parse_reducer_error(reducer: &str, status: u16, body: &str) -> ExperimentError {
    // The hexflo-coordination reducers return strings like:
    //   "Objective 'obj-1' not found"
    //   "Hypothesis 'hyp-1' not found"
    //   "Parent objective 'parent-x' not found"
    //   "Invalid status 'foo'. Must be one of: ..."
    let lower = body.to_ascii_lowercase();
    if lower.contains("objective") && lower.contains("not found") {
        if let Some(id) = extract_quoted(body) {
            return ExperimentError::ObjectiveNotFound(ObjectiveId(id));
        }
    }
    if lower.contains("hypothesis") && lower.contains("not found") {
        if let Some(id) = extract_quoted(body) {
            return ExperimentError::HypothesisNotFound(HypothesisId(id));
        }
    }
    ExperimentError::Backend(format!("reducer '{reducer}' returned {status}: {body}"))
}

fn extract_quoted(s: &str) -> Option<String> {
    let start = s.find('\'')?;
    let rest = &s[start + 1..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

// ── Enum ↔ string mappings ────────────────────────────────────

fn priority_to_str(p: &ObjectivePriority) -> &'static str {
    match p {
        ObjectivePriority::Critical => "critical",
        ObjectivePriority::High => "high",
        ObjectivePriority::Medium => "medium",
        ObjectivePriority::Low => "low",
    }
}

fn priority_from_str(s: &str) -> Result<ObjectivePriority, ExperimentError> {
    match s {
        "critical" => Ok(ObjectivePriority::Critical),
        "high" => Ok(ObjectivePriority::High),
        "medium" => Ok(ObjectivePriority::Medium),
        "low" => Ok(ObjectivePriority::Low),
        other => Err(ExperimentError::Backend(format!("invalid priority: {other}"))),
    }
}

fn comparison_to_pair(c: &ComparisonOperator) -> (&'static str, f64) {
    match c {
        ComparisonOperator::GreaterThan => ("greater_than", 0.0),
        ComparisonOperator::GreaterThanOrEqual => ("greater_than_or_equal", 0.0),
        ComparisonOperator::LessThan => ("less_than", 0.0),
        ComparisonOperator::LessThanOrEqual => ("less_than_or_equal", 0.0),
        ComparisonOperator::Equal => ("equal", 0.0),
        ComparisonOperator::WithinRange { tolerance } => ("within_range", *tolerance),
    }
}

fn comparison_from_pair(s: &str, tol: f64) -> Result<ComparisonOperator, ExperimentError> {
    match s {
        "greater_than" => Ok(ComparisonOperator::GreaterThan),
        "greater_than_or_equal" => Ok(ComparisonOperator::GreaterThanOrEqual),
        "less_than" => Ok(ComparisonOperator::LessThan),
        "less_than_or_equal" => Ok(ComparisonOperator::LessThanOrEqual),
        "equal" => Ok(ComparisonOperator::Equal),
        "within_range" => Ok(ComparisonOperator::WithinRange { tolerance: tol }),
        other => Err(ExperimentError::Backend(format!("invalid comparison: {other}"))),
    }
}

fn objective_status_to_str(s: &ObjectiveStatus) -> &'static str {
    match s {
        ObjectiveStatus::Active => "active",
        ObjectiveStatus::Achieved => "achieved",
        ObjectiveStatus::Abandoned => "abandoned",
        ObjectiveStatus::Superseded => "superseded",
    }
}

fn objective_status_from_str(s: &str) -> Result<ObjectiveStatus, ExperimentError> {
    match s {
        "active" => Ok(ObjectiveStatus::Active),
        "achieved" => Ok(ObjectiveStatus::Achieved),
        "abandoned" => Ok(ObjectiveStatus::Abandoned),
        "superseded" => Ok(ObjectiveStatus::Superseded),
        other => Err(ExperimentError::Backend(format!("invalid objective status: {other}"))),
    }
}

/// Returns (status_string, status_at, status_reason) — matches the reducer signature
/// `hypothesis_update_status(id, status, status_at, status_reason)`.
fn hypothesis_status_to_triple(s: &HypothesisStatus) -> (&'static str, String, String) {
    match s {
        HypothesisStatus::Untested => ("untested", String::new(), String::new()),
        HypothesisStatus::Confirmed { confirmed_at } => {
            ("confirmed", confirmed_at.clone(), String::new())
        }
        HypothesisStatus::Rejected { rejected_at, reason } => {
            ("rejected", rejected_at.clone(), reason.clone())
        }
        HypothesisStatus::Inconclusive { reviewed_at } => {
            ("inconclusive", reviewed_at.clone(), String::new())
        }
    }
}

fn hypothesis_status_from_triple(
    status: &str,
    status_at: &str,
    reason: &str,
) -> Result<HypothesisStatus, ExperimentError> {
    match status {
        "untested" => Ok(HypothesisStatus::Untested),
        "confirmed" => Ok(HypothesisStatus::Confirmed {
            confirmed_at: status_at.to_string(),
        }),
        "rejected" => Ok(HypothesisStatus::Rejected {
            rejected_at: status_at.to_string(),
            reason: reason.to_string(),
        }),
        "inconclusive" => Ok(HypothesisStatus::Inconclusive {
            reviewed_at: status_at.to_string(),
        }),
        other => Err(ExperimentError::Backend(format!("invalid hypothesis status: {other}"))),
    }
}

/// Returns (decision_string, decision_until, decision_reason) — matches the
/// reducer signature for the trailing three columns.
fn verdict_decision_to_triple(d: &VerdictDecision) -> (&'static str, String, String) {
    match d {
        VerdictDecision::Graduate => ("graduate", String::new(), String::new()),
        VerdictDecision::Hold { until } => ("hold", until.clone(), String::new()),
        VerdictDecision::Rollback { reason } => ("rollback", String::new(), reason.clone()),
        VerdictDecision::Inconclusive => ("inconclusive", String::new(), String::new()),
    }
}

fn verdict_decision_from_triple(
    decision: &str,
    until: &str,
    reason: &str,
) -> Result<VerdictDecision, ExperimentError> {
    match decision {
        "graduate" => Ok(VerdictDecision::Graduate),
        "hold" => Ok(VerdictDecision::Hold { until: until.to_string() }),
        "rollback" => Ok(VerdictDecision::Rollback { reason: reason.to_string() }),
        "inconclusive" => Ok(VerdictDecision::Inconclusive),
        other => Err(ExperimentError::Backend(format!("invalid verdict decision: {other}"))),
    }
}

// ── Row → domain conversions ──────────────────────────────────

fn col_str(cols: &[Value], idx: usize) -> String {
    cols.get(idx).and_then(|v| v.as_str()).unwrap_or("").to_string()
}

fn col_f64(cols: &[Value], idx: usize) -> f64 {
    cols.get(idx).and_then(|v| v.as_f64()).unwrap_or(0.0)
}

fn objective_row_to_domain(cols: &[Value]) -> Result<Objective, ExperimentError> {
    // Column order: id, project_id, name, description, parent_id, priority,
    //               target_value, comparison, comparison_tolerance, unit,
    //               status, created_at, updated_at
    let parent_id = col_str(cols, 4);
    Ok(Objective {
        id: ObjectiveId(col_str(cols, 0)),
        name: col_str(cols, 2),
        description: col_str(cols, 3),
        parent: if parent_id.is_empty() { None } else { Some(ObjectiveId(parent_id)) },
        priority: priority_from_str(&col_str(cols, 5))?,
        target_value: col_f64(cols, 6),
        comparison: comparison_from_pair(&col_str(cols, 7), col_f64(cols, 8))?,
        unit: col_str(cols, 9),
        status: objective_status_from_str(&col_str(cols, 10))?,
        created_at: col_str(cols, 11),
        updated_at: col_str(cols, 12),
    })
}

fn hypothesis_row_to_domain(cols: &[Value]) -> Result<Hypothesis, ExperimentError> {
    // Column order: id, project_id, content, target_objective_id,
    //               predicted_delta, predicted_confidence, verification_plan,
    //               adr_id, status, status_at, status_reason, created_at
    let adr_id = col_str(cols, 7);
    Ok(Hypothesis {
        id: HypothesisId(col_str(cols, 0)),
        content: col_str(cols, 2),
        target_objective: ObjectiveId(col_str(cols, 3)),
        predicted_delta: col_f64(cols, 4),
        predicted_confidence: col_f64(cols, 5),
        verification_plan: col_str(cols, 6),
        adr_id: if adr_id.is_empty() { None } else { Some(adr_id) },
        status: hypothesis_status_from_triple(
            &col_str(cols, 8),
            &col_str(cols, 9),
            &col_str(cols, 10),
        )?,
        created_at: col_str(cols, 11),
    })
}

fn verdict_row_to_domain(cols: &[Value]) -> Result<Verdict, ExperimentError> {
    // Column order: id, project_id, trial_id, hypothesis_id, objective_id,
    //               baseline_score, trial_score, delta, confidence,
    //               decision, decision_until, decision_reason,
    //               archived_at, notes
    Ok(Verdict {
        id: VerdictId(col_str(cols, 0)),
        trial_id: col_str(cols, 2),
        hypothesis_id: HypothesisId(col_str(cols, 3)),
        objective_id: ObjectiveId(col_str(cols, 4)),
        baseline_score: col_f64(cols, 5),
        trial_score: col_f64(cols, 6),
        delta: col_f64(cols, 7),
        confidence: col_f64(cols, 8),
        decision: verdict_decision_from_triple(
            &col_str(cols, 9),
            &col_str(cols, 10),
            &col_str(cols, 11),
        )?,
        archived_at: col_str(cols, 12),
        notes: col_str(cols, 13),
    })
}

// ── IExperimentPort impl ──────────────────────────────────────

#[async_trait]
impl IExperimentPort for SpacetimeExperimentAdapter {
    async fn objective_create(
        &self,
        project_id: &str,
        obj: Objective,
    ) -> Result<ObjectiveId, ExperimentError> {
        let (cmp, tol) = comparison_to_pair(&obj.comparison);
        let parent_id = obj.parent.as_ref().map(|p| p.0.clone()).unwrap_or_default();
        self.call_reducer(
            "objective_create",
            serde_json::json!([
                obj.id.0,
                project_id,
                obj.name,
                obj.description,
                parent_id,
                priority_to_str(&obj.priority),
                obj.target_value,
                cmp,
                tol,
                obj.unit,
                obj.created_at,
            ]),
        )
        .await?;
        Ok(obj.id)
    }

    async fn objective_get(
        &self,
        id: &ObjectiveId,
    ) -> Result<Option<Objective>, ExperimentError> {
        let q = format!(
            "SELECT id, project_id, name, description, parent_id, priority, \
             target_value, comparison, comparison_tolerance, unit, status, \
             created_at, updated_at FROM objective WHERE id = '{}'",
            Self::escape_sql(&id.0)
        );
        let rows = self.sql_query(&q).await?;
        match rows.first().and_then(|r| r.as_array()) {
            Some(cols) => Ok(Some(objective_row_to_domain(cols)?)),
            None => Ok(None),
        }
    }

    async fn objective_list(
        &self,
        project_id: &str,
    ) -> Result<Vec<Objective>, ExperimentError> {
        let q = format!(
            "SELECT id, project_id, name, description, parent_id, priority, \
             target_value, comparison, comparison_tolerance, unit, status, \
             created_at, updated_at FROM objective WHERE project_id = '{}'",
            Self::escape_sql(project_id)
        );
        let rows = self.sql_query(&q).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            if let Some(cols) = row.as_array() {
                out.push(objective_row_to_domain(cols)?);
            }
        }
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Ok(out)
    }

    async fn objective_update_status(
        &self,
        id: &ObjectiveId,
        status: ObjectiveStatus,
    ) -> Result<(), ExperimentError> {
        let now = chrono::Utc::now().to_rfc3339();
        self.call_reducer(
            "objective_update_status",
            serde_json::json!([id.0, objective_status_to_str(&status), now]),
        )
        .await
    }

    async fn hypothesis_create(
        &self,
        project_id: &str,
        h: Hypothesis,
    ) -> Result<HypothesisId, ExperimentError> {
        self.call_reducer(
            "hypothesis_create",
            serde_json::json!([
                h.id.0,
                project_id,
                h.content,
                h.target_objective.0,
                h.predicted_delta,
                h.predicted_confidence,
                h.verification_plan,
                h.adr_id.clone().unwrap_or_default(),
                h.created_at,
            ]),
        )
        .await?;
        Ok(h.id)
    }

    async fn hypothesis_get(
        &self,
        id: &HypothesisId,
    ) -> Result<Option<Hypothesis>, ExperimentError> {
        let q = format!(
            "SELECT id, project_id, content, target_objective_id, predicted_delta, \
             predicted_confidence, verification_plan, adr_id, status, status_at, \
             status_reason, created_at FROM hypothesis WHERE id = '{}'",
            Self::escape_sql(&id.0)
        );
        let rows = self.sql_query(&q).await?;
        match rows.first().and_then(|r| r.as_array()) {
            Some(cols) => Ok(Some(hypothesis_row_to_domain(cols)?)),
            None => Ok(None),
        }
    }

    async fn hypothesis_list_for_objective(
        &self,
        target: &ObjectiveId,
    ) -> Result<Vec<Hypothesis>, ExperimentError> {
        let q = format!(
            "SELECT id, project_id, content, target_objective_id, predicted_delta, \
             predicted_confidence, verification_plan, adr_id, status, status_at, \
             status_reason, created_at FROM hypothesis WHERE target_objective_id = '{}'",
            Self::escape_sql(&target.0)
        );
        let rows = self.sql_query(&q).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            if let Some(cols) = row.as_array() {
                out.push(hypothesis_row_to_domain(cols)?);
            }
        }
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Ok(out)
    }

    async fn hypothesis_update_status(
        &self,
        id: &HypothesisId,
        status: HypothesisStatus,
    ) -> Result<(), ExperimentError> {
        let (s, at, reason) = hypothesis_status_to_triple(&status);
        self.call_reducer(
            "hypothesis_update_status",
            serde_json::json!([id.0, s, at, reason]),
        )
        .await
    }

    async fn verdict_record(
        &self,
        project_id: &str,
        v: Verdict,
    ) -> Result<VerdictId, ExperimentError> {
        let (decision, until, reason) = verdict_decision_to_triple(&v.decision);
        self.call_reducer(
            "verdict_record",
            serde_json::json!([
                v.id.0,
                project_id,
                v.trial_id,
                v.hypothesis_id.0,
                v.objective_id.0,
                v.baseline_score,
                v.trial_score,
                v.delta,
                v.confidence,
                decision,
                until,
                reason,
                v.archived_at,
                v.notes,
            ]),
        )
        .await?;
        Ok(v.id)
    }

    async fn verdict_get(
        &self,
        id: &VerdictId,
    ) -> Result<Option<Verdict>, ExperimentError> {
        let q = format!(
            "SELECT id, project_id, trial_id, hypothesis_id, objective_id, \
             baseline_score, trial_score, delta, confidence, decision, \
             decision_until, decision_reason, archived_at, notes \
             FROM verdict WHERE id = '{}'",
            Self::escape_sql(&id.0)
        );
        let rows = self.sql_query(&q).await?;
        match rows.first().and_then(|r| r.as_array()) {
            Some(cols) => Ok(Some(verdict_row_to_domain(cols)?)),
            None => Ok(None),
        }
    }

    async fn verdict_list_for_objective(
        &self,
        obj: &ObjectiveId,
    ) -> Result<Vec<Verdict>, ExperimentError> {
        let q = format!(
            "SELECT id, project_id, trial_id, hypothesis_id, objective_id, \
             baseline_score, trial_score, delta, confidence, decision, \
             decision_until, decision_reason, archived_at, notes \
             FROM verdict WHERE objective_id = '{}'",
            Self::escape_sql(&obj.0)
        );
        let rows = self.sql_query(&q).await?;
        let mut out = Vec::with_capacity(rows.len());
        for row in &rows {
            if let Some(cols) = row.as_array() {
                out.push(verdict_row_to_domain(cols)?);
            }
        }
        out.sort_by(|a, b| a.id.0.cmp(&b.id.0));
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn priority_round_trip() {
        for p in [
            ObjectivePriority::Critical,
            ObjectivePriority::High,
            ObjectivePriority::Medium,
            ObjectivePriority::Low,
        ] {
            let s = priority_to_str(&p);
            assert_eq!(priority_from_str(s).unwrap(), p);
        }
    }

    #[test]
    fn comparison_round_trip_with_tolerance() {
        let c = ComparisonOperator::WithinRange { tolerance: 1.5 };
        let (s, tol) = comparison_to_pair(&c);
        assert_eq!(s, "within_range");
        assert_eq!(tol, 1.5);
        assert_eq!(comparison_from_pair(s, tol).unwrap(), c);
    }

    #[test]
    fn comparison_non_range_uses_zero_tol() {
        for c in [
            ComparisonOperator::GreaterThan,
            ComparisonOperator::LessThan,
            ComparisonOperator::Equal,
        ] {
            let (s, tol) = comparison_to_pair(&c);
            assert_eq!(tol, 0.0);
            assert_eq!(comparison_from_pair(s, 0.0).unwrap(), c);
        }
    }

    #[test]
    fn hypothesis_status_round_trip_rejected() {
        let st = HypothesisStatus::Rejected {
            rejected_at: "2026-05-02T00:00:00Z".into(),
            reason: "p > 0.05".into(),
        };
        let (s, at, r) = hypothesis_status_to_triple(&st);
        assert_eq!(s, "rejected");
        assert_eq!(hypothesis_status_from_triple(s, &at, &r).unwrap(), st);
    }

    #[test]
    fn verdict_decision_round_trip_hold() {
        let d = VerdictDecision::Hold { until: "2026-06-01T00:00:00Z".into() };
        let (s, until, reason) = verdict_decision_to_triple(&d);
        assert_eq!(s, "hold");
        assert_eq!(reason, "");
        assert_eq!(verdict_decision_from_triple(s, &until, &reason).unwrap(), d);
    }

    #[test]
    fn parse_objective_not_found() {
        let err = parse_reducer_error(
            "verdict_record",
            400,
            "Objective 'obj-xyz' not found",
        );
        match err {
            ExperimentError::ObjectiveNotFound(id) => assert_eq!(id.0, "obj-xyz"),
            other => panic!("expected ObjectiveNotFound, got {other:?}"),
        }
    }

    #[test]
    fn parse_hypothesis_not_found() {
        let err = parse_reducer_error(
            "verdict_record",
            400,
            "Hypothesis 'hyp-1' not found",
        );
        match err {
            ExperimentError::HypothesisNotFound(id) => assert_eq!(id.0, "hyp-1"),
            other => panic!("expected HypothesisNotFound, got {other:?}"),
        }
    }

    #[test]
    fn parse_unknown_falls_through_to_backend() {
        let err = parse_reducer_error("objective_create", 500, "internal explosion");
        assert!(matches!(err, ExperimentError::Backend(_)));
    }

    #[test]
    fn objective_row_decodes_no_parent() {
        let cols = vec![
            Value::String("o-1".into()),
            Value::String("p-1".into()),
            Value::String("name".into()),
            Value::String("desc".into()),
            Value::String("".into()),
            Value::String("high".into()),
            serde_json::json!(100.0),
            Value::String("less_than".into()),
            serde_json::json!(0.0),
            Value::String("ms".into()),
            Value::String("active".into()),
            Value::String("2026-05-02T00:00:00Z".into()),
            Value::String("2026-05-02T00:00:00Z".into()),
        ];
        let obj = objective_row_to_domain(&cols).unwrap();
        assert_eq!(obj.id.0, "o-1");
        assert!(obj.parent.is_none());
        assert_eq!(obj.priority, ObjectivePriority::High);
        assert_eq!(obj.comparison, ComparisonOperator::LessThan);
    }

    #[test]
    fn dyn_safety() {
        // SpacetimeDB calls are network-bound — exercise the type system only.
        fn _assert_dyn(_: &dyn IExperimentPort) {}
        let adapter = SpacetimeExperimentAdapter::new("http://x".into(), "y".into());
        _assert_dyn(&adapter);
    }
}
