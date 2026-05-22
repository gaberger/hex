//! End-to-end classifier pipeline integration test for the SOP pipeline
//! redesign Phase 1 (ADR-2026-05-17-2030, workplan
//! `wp-sop-pipeline-redesign-phase-1`, task P6.1).
//!
//! Exercises the full ingest→dispatch path:
//!
//!   1. A canned LLM response is fed through the **real**
//!      `StrictJsonClassifierAdapter` (P3.1) via a [`MockInferencePort`].
//!   2. The parsed [`ClassifierResponse`] (or [`InvariantError`]) is then
//!      routed through the same primitives `org_responder::process_role`
//!      uses post-classify: `post_classifier_response_open` for the STDB
//!      row, `route_decision` for the dispatch fan-out, and
//!      `escalate_classifier_failure` for the operator-inbox path.
//!   3. Side-effects (STDB reducer calls, send_dm, mark_read) are observed
//!      via an `httpmock` server playing the role of SpacetimeDB.
//!
//! Coverage matrix — 6 `ClassifierDecision` variants × `from ∈ {operator,
//! peer}` × the schema invariant cases for `from=operator` Defer/Reject
//! (which become [`InvariantError::DecisionNotAllowedForOperator`]):
//!
//! | decision      | from=operator path       | from=peer path           |
//! |---------------|--------------------------|--------------------------|
//! | accept        | classifier_response_open + send_dm reply        | same |
//! | defer         | INVARIANT → escalation (no reply)               | classifier_response_open + send_dm |
//! | route         | classifier_response_open + forward_dm + reply   | same |
//! | clarify       | classifier_response_open + reply (with question)| same |
//! | reject        | INVARIANT → escalation (no reply)               | classifier_response_open + send_dm |
//! | request_tool  | classifier_response_open + notify_agent + reply | same |
//!
//! Final acceptance assertion: across all 12 cases, every inbound DM
//! produces an observable artifact (a classifier_response row or an
//! escalation). Zero silent drops — this is the Phase 1 acceptance gate.
//!
//! ## Why we mock STDB at the HTTP layer
//!
//! Both `post_classifier_response_open` and `post_inbox_notify` post to
//! `/v1/database/<db>/call/<reducer>` directly via `reqwest` (see
//! `stdb_endpoint()` in `org_responder.rs`). The dispatch primitives
//! (`route_decision`, `escalate_classifier_failure`,
//! `post_classifier_response_open`) all pull their host/db from env vars
//! at call time, so a per-test `httpmock` server + scoped env-var swap is
//! the smallest surface that exercises the real I/O code path without
//! standing up a SpacetimeDB instance. The `SpacetimeAgentCommAdapter`
//! constructor takes host+db directly — we point it at the same mock URL.

use std::sync::Arc;

use hex_core::ports::agent_comm::IAgentCommPort;
use hex_core::ports::inference::{IInferencePort, mock::MockInferencePort};
use hex_nexus::adapters::spacetime_agent_comm::SpacetimeAgentCommAdapter;
use hex_nexus::orchestration::classifier_adapter::StrictJsonClassifierAdapter;
use hex_nexus::orchestration::classifier_parser::InvariantError;
use hex_nexus::orchestration::classifier_types::{ClassifierDecision, ClassifierResponse};
use hex_nexus::orchestration::org_responder::{
    decision_str, escalate_classifier_failure, post_classifier_response_open, route_decision,
};

use httpmock::prelude::*;
use std::sync::OnceLock;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Test harness: process-wide env lock + helpers
// ---------------------------------------------------------------------------
//
// `post_classifier_response_open` and `post_inbox_notify` resolve the STDB
// host + database from process-wide env vars at call time
// (HEX_SPACETIMEDB_HOST / HEX_STDB_DATABASE). Cargo runs integration tests
// inside a single binary with `--test-threads` ≥ 1; without a shared mutex
// two tests racing on the env can clobber each other's mock server URL.
// Acquire this lock before mutating either var.

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// RAII guard restoring env vars on drop. Held for the duration of each
/// test case so concurrently-running cases don't see each other's STDB URL.
struct EnvGuard {
    saved: Vec<(&'static str, Option<String>)>,
}

impl EnvGuard {
    fn capture(keys: &[&'static str]) -> Self {
        let saved = keys.iter().map(|k| (*k, std::env::var(k).ok())).collect();
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(val) => std::env::set_var(k, val),
                None => std::env::remove_var(k),
            }
        }
    }
}

const TEST_DB: &str = "classifier_pipeline_test";
const ENV_KEYS: &[&str] = &["HEX_SPACETIMEDB_HOST", "HEX_STDB_DATABASE"];

/// Mount a permissive 200-OK match for ANY `/v1/database/<db>/call/<reducer>`
/// POST so unmatched reducer calls don't 404 and break the helper's tail
/// `mark_read` step. Specific matchers mounted BEFORE this one win.
fn mount_catchall(server: &MockServer) {
    server.mock(|when, then| {
        when.method(POST)
            .path_matches(regex::Regex::new(r"^/v1/database/[^/]+/call/.*$").unwrap());
        then.status(200).body("");
    });
}

/// Match a `classifier_response_open` POST with the expected
/// `(final_outcome, reparse_attempts, decision)` triple. The reducer body
/// is positional JSON; we match on substrings to stay tolerant of key
/// ordering and adjacent fields.
fn mount_classifier_open<'a>(
    server: &'a MockServer,
    expected_outcome: &str,
    expected_attempts: u32,
    expected_decision: &str,
) -> httpmock::Mock<'a> {
    let outcome = format!(",\"{expected_outcome}\",");
    let attempts = format!(",{expected_attempts},");
    let decision = format!(",\"{expected_decision}\",");
    server.mock(|when, then| {
        when.method(POST)
            .path_matches(
                regex::Regex::new(r"^/v1/database/[^/]+/call/classifier_response_open$").unwrap(),
            )
            .body_contains(outcome)
            .body_contains(attempts)
            .body_contains(decision);
        then.status(200).body("");
    })
}

/// Match a `notify_agent` POST to the operator with priority=2 + the
/// given kind. Used to verify the `request_tool` and
/// `classifier_escalation` inbox notifications.
fn mount_notify<'a>(server: &'a MockServer, expected_kind: &str) -> httpmock::Mock<'a> {
    let kind_marker = format!("\"{expected_kind}\"");
    server.mock(|when, then| {
        when.method(POST)
            .path_matches(regex::Regex::new(r"^/v1/database/[^/]+/call/notify_agent$").unwrap())
            .body_contains("\"operator\"")
            .body_contains(",2,")
            .body_contains(kind_marker);
        then.status(200).body("");
    })
}

/// Match a `send_dm` POST from `from_role` to `to_role`. Used to verify
/// the per-decision reply send and the `Route` forward.
fn mount_send_dm<'a>(server: &'a MockServer, from_role: &str, to_role: &str) -> httpmock::Mock<'a> {
    // SpacetimeAgentCommAdapter::send_dm posts a positional array
    // `[from, to, message, thread_option]`. We match by substring on the
    // first two so the test stays tolerant of the message body and the
    // Option<String> sum-type encoding.
    let from_marker = format!("\"{from_role}\"");
    let to_marker = format!("\"{to_role}\"");
    server.mock(|when, then| {
        when.method(POST)
            .path_matches(regex::Regex::new(r"^/v1/database/[^/]+/call/send_dm$").unwrap())
            .body_contains(from_marker)
            .body_contains(to_marker);
        then.status(200).body("");
    })
}

/// Match the `mark_read` reducer for `(role, msg_id)`. Fires in the
/// escalation path so the 4 s responder tick doesn't refire on the same
/// inbound DM.
fn mount_mark_read<'a>(server: &'a MockServer, role: &str, msg_id: u64) -> httpmock::Mock<'a> {
    let role_marker = format!("\"{role}\"");
    let id_marker = format!(",{msg_id}");
    server.mock(|when, then| {
        when.method(POST)
            .path_matches(regex::Regex::new(r"^/v1/database/[^/]+/call/mark_read$").unwrap())
            .body_contains(role_marker)
            .body_contains(id_marker);
        then.status(200).body("");
    })
}

fn build_comm(server_url: &str) -> Arc<SpacetimeAgentCommAdapter> {
    Arc::new(SpacetimeAgentCommAdapter::new(
        server_url.to_string(),
        TEST_DB.to_string(),
    ))
}

fn set_stdb_env(server_url: &str) {
    std::env::set_var("HEX_SPACETIMEDB_HOST", server_url);
    std::env::set_var("HEX_STDB_DATABASE", TEST_DB);
}

// ---------------------------------------------------------------------------
// Canned LLM responses — one per ClassifierDecision variant.
//
// These are the same wire-format JSON shapes the real persona models emit;
// they round-trip through `SerdeJsonClassifierParser` (P1.2) inside the
// `StrictJsonClassifierAdapter::classify_with_attempts` call.
// ---------------------------------------------------------------------------

const RESP_ACCEPT: &str = r#"{"decision":"accept","tool_plan":[{"tool":"code_patch","intent":"patch the dispatcher"}],"cost_usd":0.0012}"#;
const RESP_DEFER: &str = r#"{"decision":"defer","reason":"blocked on STDB outage","cost_usd":0.0008}"#;
const RESP_ROUTE: &str = r#"{"decision":"route","target_persona":"ciso","cost_usd":0.0009}"#;
const RESP_CLARIFY: &str =
    r#"{"decision":"clarify","question":"Which workplan should I target?","cost_usd":0.0007}"#;
const RESP_REJECT: &str = r#"{"decision":"reject","reason":"out of persona scope","cost_usd":0.0006}"#;
const RESP_REQUEST_TOOL: &str = r#"{"decision":"request_tool","tool_spec":{"name":"grep_workplan","rationale":"need wp dep lookups"},"cost_usd":0.0011}"#;

/// Run the classifier adapter against `canned_llm_output`, then drive the
/// post-classify dispatch path identically to `org_responder::process_role`.
/// Returns the parsed response on success, or the invariant error if the
/// classifier escalated.
///
/// `from_role` is "operator" or "peer". For `from_role="operator"` +
/// decision ∈ {defer, reject}, the parser raises
/// `InvariantError::DecisionNotAllowedForOperator` and the caller-side
/// branch routes through `escalate_classifier_failure` instead of the
/// success path.
async fn drive_pipeline(
    server_url: &str,
    canned_llm_output: &str,
    from_role: &str,
    role: &str,
    msg_id: u64,
    original_content: &str,
    thread_id: Option<&str>,
) -> Result<ClassifierResponse, InvariantError> {
    let inference: Arc<dyn IInferencePort> = Arc::new(MockInferencePort::with_response(
        canned_llm_output.to_string(),
    ));
    let classifier = StrictJsonClassifierAdapter::new(inference, "mock");

    let comm = build_comm(server_url);
    let from_operator = from_role == "operator";

    let result = classifier
        .classify_with_attempts("you are a classifier", original_content, from_operator)
        .await;

    match result {
        Ok((resp, attempts)) => {
            // Success path — mirror process_role: persist the row, then route.
            let tool_plan_json =
                serde_json::to_string(&resp.tool_plan).unwrap_or_else(|_| "null".to_string());
            let tool_spec_json =
                serde_json::to_string(&resp.tool_spec).unwrap_or_else(|_| "null".to_string());

            post_classifier_response_open(
                msg_id,
                from_role,
                role,
                decision_str(&resp.decision),
                &tool_plan_json,
                resp.reason.as_deref().unwrap_or(""),
                resp.target_persona.as_deref().unwrap_or(""),
                resp.question.as_deref().unwrap_or(""),
                &tool_spec_json,
                attempts,
                "parsed",
                resp.cost_usd,
            )
            .await;

            let reply = route_decision(
                &comm,
                role,
                role,
                from_role,
                original_content,
                thread_id,
                &resp,
                msg_id,
            )
            .await;

            // process_role sends the reply back to `from`. Replicate so the
            // send_dm matcher fires.
            let _ = comm
                .send_dm(
                    role.to_string(),
                    from_role.to_string(),
                    reply,
                    thread_id.map(|s| s.to_string()),
                )
                .await;

            Ok(resp)
        }
        Err(invariant) => {
            // Failure path — escalate. The helper writes the
            // `classifier_response_open` row + notify_agent + mark_read.
            escalate_classifier_failure(
                &comm,
                role,
                msg_id,
                from_role,
                role,
                original_content,
                thread_id,
                &invariant,
            )
            .await;
            Err(invariant)
        }
    }
}

// ---------------------------------------------------------------------------
// Decision × from-role test cases
// ---------------------------------------------------------------------------

// --- accept ---------------------------------------------------------------

#[tokio::test]
async fn accept_from_operator_writes_parsed_row_and_sends_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "accept");
    let reply = mount_send_dm(&server, "cto", "operator");
    mount_catchall(&server);

    let resp = drive_pipeline(
        &server.base_url(),
        RESP_ACCEPT,
        "operator",
        "cto",
        101,
        "ship the migration",
        None,
    )
    .await
    .expect("accept from operator should parse");
    assert!(matches!(resp.decision, ClassifierDecision::Accept));

    assert_eq!(row.hits(), 1, "expected one classifier_response_open(parsed, 1, accept)");
    assert_eq!(reply.hits(), 1, "expected one send_dm reply back to operator");
}

#[tokio::test]
async fn accept_from_peer_writes_parsed_row_and_sends_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "accept");
    let reply = mount_send_dm(&server, "cto", "ciso");
    mount_catchall(&server);

    drive_pipeline(
        &server.base_url(),
        RESP_ACCEPT,
        "ciso",
        "cto",
        102,
        "patch the dispatcher",
        Some("peer-thread"),
    )
    .await
    .expect("accept from peer should parse");

    assert_eq!(row.hits(), 1);
    assert_eq!(reply.hits(), 1);
}

// --- defer ----------------------------------------------------------------

#[tokio::test]
async fn defer_from_operator_escalates_as_invariant_violation() {
    // from=operator + decision=defer is forbidden by the parser — must
    // route to escalation (final_outcome=invariant_violation, attempts=1,
    // decision_label=defer) and NOT send a reply.
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "invariant_violation", 1, "defer");
    let inbox = mount_notify(&server, "classifier_escalation");
    let read = mount_mark_read(&server, "cto", 201);
    mount_catchall(&server);

    let err = drive_pipeline(
        &server.base_url(),
        RESP_DEFER,
        "operator",
        "cto",
        201,
        "schedule the migration later",
        None,
    )
    .await
    .expect_err("operator+defer must violate invariant");
    assert!(matches!(
        err,
        InvariantError::DecisionNotAllowedForOperator(ClassifierDecision::Defer)
    ));

    assert_eq!(row.hits(), 1, "expected invariant_violation row tagged as 'defer'");
    assert_eq!(inbox.hits(), 1, "expected operator inbox escalation");
    assert_eq!(read.hits(), 1, "expected mark_read to break the retry loop");
}

#[tokio::test]
async fn defer_from_peer_writes_parsed_row_and_sends_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "defer");
    let reply = mount_send_dm(&server, "cpo", "cto");
    mount_catchall(&server);

    drive_pipeline(
        &server.base_url(),
        RESP_DEFER,
        "cto",
        "cpo",
        202,
        "draft the pricing spec",
        None,
    )
    .await
    .expect("defer from peer is allowed");

    assert_eq!(row.hits(), 1);
    assert_eq!(reply.hits(), 1);
}

// --- route ----------------------------------------------------------------

#[tokio::test]
async fn route_from_operator_forwards_to_target_and_sends_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "route");
    // route_decision forwards the original ask: send_dm(from=cto, to=ciso)
    let forward = mount_send_dm(&server, "cto", "ciso");
    // ...and the responder sends the reply text back to the operator.
    let reply = mount_send_dm(&server, "cto", "operator");
    mount_catchall(&server);

    let resp = drive_pipeline(
        &server.base_url(),
        RESP_ROUTE,
        "operator",
        "cto",
        301,
        "audit the auth path",
        None,
    )
    .await
    .expect("route from operator should parse");
    assert!(matches!(resp.decision, ClassifierDecision::Route));
    assert_eq!(resp.target_persona.as_deref(), Some("ciso"));

    assert_eq!(row.hits(), 1);
    // Forward + reply both go through send_dm. The body-substring matchers
    // (`from=cto` + `to=ciso` vs `from=cto` + `to=operator`) discriminate.
    assert_eq!(forward.hits(), 1, "expected one forward_dm to target_persona");
    assert_eq!(reply.hits(), 1, "expected one reply send_dm to source");
}

#[tokio::test]
async fn route_from_peer_forwards_to_target_and_sends_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "route");
    let forward = mount_send_dm(&server, "cto", "ciso");
    let reply = mount_send_dm(&server, "cto", "cpo");
    mount_catchall(&server);

    drive_pipeline(
        &server.base_url(),
        RESP_ROUTE,
        "cpo",
        "cto",
        302,
        "review the security posture",
        None,
    )
    .await
    .expect("route from peer should parse");

    assert_eq!(row.hits(), 1);
    assert_eq!(forward.hits(), 1);
    assert_eq!(reply.hits(), 1);
}

// --- clarify --------------------------------------------------------------

#[tokio::test]
async fn clarify_from_operator_sends_question_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "clarify");
    let reply = mount_send_dm(&server, "cto", "operator");
    mount_catchall(&server);

    let resp = drive_pipeline(
        &server.base_url(),
        RESP_CLARIFY,
        "operator",
        "cto",
        401,
        "the workplan thing",
        None,
    )
    .await
    .expect("clarify from operator should parse");
    assert!(matches!(resp.decision, ClassifierDecision::Clarify));
    assert!(resp.question.is_some());

    assert_eq!(row.hits(), 1);
    assert_eq!(reply.hits(), 1);
}

#[tokio::test]
async fn clarify_from_peer_sends_question_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "clarify");
    let reply = mount_send_dm(&server, "cto", "ciso");
    mount_catchall(&server);

    drive_pipeline(
        &server.base_url(),
        RESP_CLARIFY,
        "ciso",
        "cto",
        402,
        "ambiguous request",
        None,
    )
    .await
    .expect("clarify from peer should parse");

    assert_eq!(row.hits(), 1);
    assert_eq!(reply.hits(), 1);
}

// --- reject ---------------------------------------------------------------

#[tokio::test]
async fn reject_from_operator_escalates_as_invariant_violation() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "invariant_violation", 1, "reject");
    let inbox = mount_notify(&server, "classifier_escalation");
    let read = mount_mark_read(&server, "ciso", 501);
    mount_catchall(&server);

    let err = drive_pipeline(
        &server.base_url(),
        RESP_REJECT,
        "operator",
        "ciso",
        501,
        "do the impossible thing",
        None,
    )
    .await
    .expect_err("operator+reject must violate invariant");
    assert!(matches!(
        err,
        InvariantError::DecisionNotAllowedForOperator(ClassifierDecision::Reject)
    ));

    assert_eq!(row.hits(), 1);
    assert_eq!(inbox.hits(), 1);
    assert_eq!(read.hits(), 1);
}

#[tokio::test]
async fn reject_from_peer_writes_parsed_row_and_sends_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "reject");
    let reply = mount_send_dm(&server, "ciso", "cto");
    mount_catchall(&server);

    drive_pipeline(
        &server.base_url(),
        RESP_REJECT,
        "cto",
        "ciso",
        502,
        "ask out of scope",
        None,
    )
    .await
    .expect("reject from peer is allowed");

    assert_eq!(row.hits(), 1);
    assert_eq!(reply.hits(), 1);
}

// --- request_tool ---------------------------------------------------------

#[tokio::test]
async fn request_tool_from_operator_notifies_inbox_and_sends_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "request_tool");
    // route_decision for RequestTool fires post_inbox_notify with
    // kind="request_tool" (NOT classifier_escalation — that's the failure
    // path).
    let inbox = mount_notify(&server, "request_tool");
    let reply = mount_send_dm(&server, "cto", "operator");
    mount_catchall(&server);

    let resp = drive_pipeline(
        &server.base_url(),
        RESP_REQUEST_TOOL,
        "operator",
        "cto",
        601,
        "I need a workplan grep tool",
        None,
    )
    .await
    .expect("request_tool from operator should parse");
    assert!(matches!(resp.decision, ClassifierDecision::RequestTool));
    assert!(resp.tool_spec.is_some());

    assert_eq!(row.hits(), 1);
    assert_eq!(inbox.hits(), 1, "expected operator inbox notify with kind=request_tool");
    assert_eq!(reply.hits(), 1);
}

#[tokio::test]
async fn request_tool_from_peer_notifies_inbox_and_sends_reply() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    let row = mount_classifier_open(&server, "parsed", 1, "request_tool");
    let inbox = mount_notify(&server, "request_tool");
    let reply = mount_send_dm(&server, "cto", "ciso");
    mount_catchall(&server);

    drive_pipeline(
        &server.base_url(),
        RESP_REQUEST_TOOL,
        "ciso",
        "cto",
        602,
        "need new tool to proceed",
        None,
    )
    .await
    .expect("request_tool from peer should parse");

    assert_eq!(row.hits(), 1);
    assert_eq!(inbox.hits(), 1);
    assert_eq!(reply.hits(), 1);
}

// ---------------------------------------------------------------------------
// Final acceptance: across all 12 cases, zero silent drops
// ---------------------------------------------------------------------------
//
// The 12 case-tests above each assert their own classifier_response_open or
// escalation. This composite test re-runs the full matrix back-to-back
// against one server and asserts the global invariant: every inbound DM
// produces exactly one classifier_response_open row OR one escalation row.
// Zero silent drops — the Phase 1 acceptance gate.

#[tokio::test]
async fn matrix_zero_silent_drops_across_six_decisions_and_two_from_roles() {
    let _lock = env_lock().lock().await;
    let _guard = EnvGuard::capture(ENV_KEYS);
    let server = MockServer::start();
    set_stdb_env(&server.base_url());

    // Single counter for ANY classifier_response_open call, regardless of
    // shape. After the full sweep this should equal the number of cases
    // (12) — proving zero silent drops.
    let row_any = server.mock(|when, then| {
        when.method(POST)
            .path_matches(
                regex::Regex::new(r"^/v1/database/[^/]+/call/classifier_response_open$").unwrap(),
            );
        then.status(200).body("");
    });
    mount_catchall(&server);

    // Cases: (canned_llm, from, role, msg_id, expects_invariant_err)
    let cases: &[(&str, &str, &str, u64, bool)] = &[
        // accept
        (RESP_ACCEPT, "operator", "cto", 1001, false),
        (RESP_ACCEPT, "ciso", "cto", 1002, false),
        // defer
        (RESP_DEFER, "operator", "cto", 1003, true),
        (RESP_DEFER, "cto", "cpo", 1004, false),
        // route
        (RESP_ROUTE, "operator", "cto", 1005, false),
        (RESP_ROUTE, "cpo", "cto", 1006, false),
        // clarify
        (RESP_CLARIFY, "operator", "cto", 1007, false),
        (RESP_CLARIFY, "ciso", "cto", 1008, false),
        // reject
        (RESP_REJECT, "operator", "ciso", 1009, true),
        (RESP_REJECT, "cto", "ciso", 1010, false),
        // request_tool
        (RESP_REQUEST_TOOL, "operator", "cto", 1011, false),
        (RESP_REQUEST_TOOL, "ciso", "cto", 1012, false),
    ];

    for (canned, from, role, msg_id, expects_err) in cases {
        let outcome = drive_pipeline(
            &server.base_url(),
            canned,
            from,
            role,
            *msg_id,
            "matrix sweep ask",
            None,
        )
        .await;
        if *expects_err {
            assert!(
                outcome.is_err(),
                "case msg_id={msg_id} (from={from}) should escalate as InvariantError"
            );
        } else {
            assert!(
                outcome.is_ok(),
                "case msg_id={msg_id} (from={from}) should parse cleanly"
            );
        }
    }

    assert_eq!(
        row_any.hits(),
        cases.len(),
        "expected one classifier_response_open row per case ({}) — zero silent drops",
        cases.len()
    );
}
