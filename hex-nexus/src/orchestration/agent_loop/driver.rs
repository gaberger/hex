//! ReAct driver for the SOP agent loop (wp-sop-agent-loop P2.2).
//!
//! Each iteration:
//!   1. Build a prompt from (system + task_brief + accumulated steps + tool descriptions)
//!   2. Inference call → string reply
//!   3. Parse `{"thought": "...", "action": {"tool": "X", "args": {...}}}`
//!      (tolerant of leading prose, leading ```json fences, trailing prose)
//!   4. Look up tool by name → invoke → record observation
//!   5. If observation has `terminal: Some(_)`, halt with TerminalAction
//!   6. Else check step + token budget, loop
//!
//! The persona contract is documented in the system prompt; off-contract
//! output (missing JSON object, missing `action`, unknown tool, parse
//! exhaustion) terminates with a structured reason so the drafter knows
//! to abandon rather than retry blind.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use hex_core::domain::messages::{ContentBlock, Message, Role};
use hex_core::ports::inference::{
    IInferencePort, InferenceRequest, Priority,
};

use crate::orchestration::agent_loop::tool::{IAgentTool, ToolError};
use crate::orchestration::agent_loop::trajectory::{AgentStep, TerminatedReason, Trajectory};

/// How many consecutive parse failures we tolerate before giving up.
/// Matches the 2-reparse retry budget in StrictJsonClassifierAdapter
/// so the operator sees consistent escalation thresholds across the
/// SOP pipeline.
const PARSE_RETRY_BUDGET: u32 = 3;

/// Inputs to a single `run()` call. Kept as a struct (vs ten args) so
/// future knobs (per-step temperature, custom system prefix) extend
/// without breaking callers.
pub struct AgentRunInput<'a> {
    pub role: &'a str,
    pub task_brief: &'a str,
    pub tools: Vec<Box<dyn IAgentTool>>,
    pub max_steps: u32,
    /// Cumulative output-token budget across all steps. Driver halts
    /// with `TokenBudget` when this is exceeded.
    pub max_output_tokens: u64,
    pub inference: Arc<dyn IInferencePort>,
    pub model: String,
}

pub async fn run(input: AgentRunInput<'_>) -> Trajectory {
    let mut trajectory = Trajectory::new(input.role, input.task_brief);

    // Index tools by name for O(1) dispatch + a stable description list
    // for the system prompt.
    let tool_by_name: HashMap<String, &dyn IAgentTool> = input
        .tools
        .iter()
        .map(|t| (t.name().to_string(), t.as_ref()))
        .collect();
    let tool_descriptions = render_tool_descriptions(&input.tools);

    let system_prompt = build_system_prompt(input.role, input.task_brief, &tool_descriptions);
    let mut parse_failures: u32 = 0;

    for step_idx in 0..input.max_steps {
        let user_text = build_user_turn(&trajectory.steps);
        let started = Instant::now();
        let req = InferenceRequest {
            model: input.model.clone(),
            system_prompt: system_prompt.clone(),
            messages: vec![Message {
                role: Role::User,
                content: vec![ContentBlock::Text { text: user_text }],
            }],
            tools: Vec::new(),
            max_tokens: 4096,
            temperature: 0.2,
            thinking_budget: None,
            cache_control: false,
            priority: Priority::Normal,
            grammar: None,
        };

        let response = match input.inference.complete(req).await {
            Ok(r) => r,
            Err(e) => {
                trajectory.terminated_reason = TerminatedReason::InferenceFailed {
                    error: format!("{}", e).chars().take(200).collect(),
                };
                return trajectory;
            }
        };
        let latency_ms = started.elapsed().as_millis() as u64;
        trajectory.total_input_tokens += response.input_tokens;
        trajectory.total_output_tokens += response.output_tokens;
        trajectory.total_latency_ms += latency_ms;
        let reply_text = extract_text(&response.content);

        let parsed = match parse_action(&reply_text) {
            Ok(p) => {
                parse_failures = 0;
                p
            }
            Err(_) => {
                parse_failures += 1;
                trajectory.steps.push(AgentStep {
                    step_idx,
                    thought: String::new(),
                    tool: String::new(),
                    args_json: String::new(),
                    observation: format!(
                        "error: response was not valid JSON object with {{thought, action: {{tool, args}}}}. Reply (first 200 chars): {}",
                        reply_text.chars().take(200).collect::<String>()
                    ),
                    bytes: reply_text.len(),
                    truncated: false,
                    output_tokens: response.output_tokens,
                    input_tokens: response.input_tokens,
                    latency_ms,
                });
                if parse_failures >= PARSE_RETRY_BUDGET {
                    trajectory.terminated_reason = TerminatedReason::ParseExhausted;
                    return trajectory;
                }
                continue;
            }
        };

        let tool = match tool_by_name.get(&parsed.tool) {
            Some(t) => *t,
            None => {
                trajectory.steps.push(AgentStep {
                    step_idx,
                    thought: parsed.thought.clone(),
                    tool: parsed.tool.clone(),
                    args_json: parsed.args.to_string(),
                    observation: format!("error: unknown tool '{}'. Available: {}",
                        parsed.tool,
                        tool_by_name.keys().cloned().collect::<Vec<_>>().join(", ")),
                    bytes: 0,
                    truncated: false,
                    output_tokens: response.output_tokens,
                    input_tokens: response.input_tokens,
                    latency_ms,
                });
                trajectory.terminated_reason = TerminatedReason::UnknownTool {
                    name: parsed.tool.clone(),
                };
                return trajectory;
            }
        };

        let invoke_result = tool.invoke(parsed.args.clone()).await;
        let (observation_text, bytes, truncated, terminal_action) = match invoke_result {
            Ok(obs) => (obs.output.clone(), obs.bytes, obs.truncated, obs.terminal),
            Err(e) => {
                // Tool error is a recoverable event — record it as the
                // observation and let the persona react on the next turn.
                let msg = format!("error: {}", e);
                let bytes = msg.len();
                (msg, bytes, false, None)
            }
        };

        trajectory.steps.push(AgentStep {
            step_idx,
            thought: parsed.thought,
            tool: parsed.tool,
            args_json: parsed.args.to_string(),
            observation: observation_text,
            bytes,
            truncated,
            output_tokens: response.output_tokens,
            input_tokens: response.input_tokens,
            latency_ms,
        });

        if let Some(action) = terminal_action {
            trajectory.final_action = Some(action);
            trajectory.terminated_reason = TerminatedReason::TerminalAction;
            return trajectory;
        }

        if trajectory.total_output_tokens >= input.max_output_tokens {
            trajectory.terminated_reason = TerminatedReason::TokenBudget;
            return trajectory;
        }
    }

    trajectory.terminated_reason = TerminatedReason::MaxSteps;
    trajectory
}

// ---------------------------------------------------------------------
// Prompt construction
// ---------------------------------------------------------------------

fn build_system_prompt(role: &str, task_brief: &str, tool_descriptions: &str) -> String {
    format!(
        "You are the `{role}` agent in a hexagonal AIOS organization, running inside a \
ReAct tool-use loop. Your task is below.

=== TASK ===
{task_brief}

=== TOOLS YOU MAY CALL ===
{tool_descriptions}

=== OUTPUT CONTRACT (HARD) ===
On every turn you MUST reply with EXACTLY ONE JSON object — no prose before, no prose \
after, no markdown fences:

  {{ \"thought\": \"<one-line reasoning about the next step>\", \
\"action\": {{ \"tool\": \"<tool name>\", \"args\": {{ ... }} }} }}

- `thought` is for your reasoning. Keep it short (<200 chars). The user sees this; it \
counts toward your output-token budget.
- `action.tool` MUST be one of the tool names listed above. Typos terminate the loop.
- `action.args` MUST conform to the tool's args schema.

To finish, call `code_patch_propose` — that is the only terminal action. Calling it \
halts the loop and submits your draft for twin review.

=== STRATEGY ===
1. Read context first. Use `repo_read` on the target file (if it exists) and any \
sibling tests / similar functions so your draft is grounded in real code, not invented \
imports.
2. When you have enough context, draft your patch and call `cargo_check` to verify \
it compiles BEFORE you `code_patch_propose`. A failing build is a signal to iterate, \
not submit.
3. Cite real file paths from observations. Do NOT invent paths.
4. Each turn is paid for in tokens. Be concise.

Begin your reply with `{{` now.")
}

fn render_tool_descriptions(tools: &[Box<dyn IAgentTool>]) -> String {
    let mut out = String::new();
    for t in tools {
        out.push_str(&format!(
            "- `{}` — {}\n  args schema: {}\n",
            t.name(),
            t.description(),
            t.schema()
        ));
    }
    out
}

fn build_user_turn(steps: &[AgentStep]) -> String {
    if steps.is_empty() {
        return "Begin. Emit your first {thought, action} JSON object now.".to_string();
    }
    let mut out = String::from("Prior steps:\n");
    for s in steps {
        out.push_str(&format!(
            "\nstep[{}] tool={} args={}\nobservation: {}\n",
            s.step_idx, s.tool, s.args_json, summarise(&s.observation, 800)
        ));
    }
    out.push_str("\nEmit your next {thought, action} JSON object now.");
    out
}

fn summarise(text: &str, max: usize) -> String {
    if text.len() <= max { return text.to_string(); }
    format!("{}... [{} more chars]", &text[..max], text.len() - max)
}

fn extract_text(content: &[ContentBlock]) -> String {
    let mut out = String::new();
    for block in content {
        if let ContentBlock::Text { text } = block {
            out.push_str(text);
        }
    }
    out
}

// ---------------------------------------------------------------------
// Reply parsing — tolerant of leading prose / fences / trailing prose
// ---------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct ParsedAction {
    pub thought: String,
    pub tool: String,
    pub args: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("no JSON object found in reply")]
    NoJsonObject,
    #[error("JSON decode: {0}")]
    Decode(String),
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

pub fn parse_action(reply: &str) -> Result<ParsedAction, ParseError> {
    // Strategy: find the first balanced `{...}` substring and try to
    // decode that. Tolerates leading prose ("Sure! Here is my reply:")
    // and leading ```json fences (we strip them by sliding past them).
    let candidate = locate_json_object(reply).ok_or(ParseError::NoJsonObject)?;

    let value: serde_json::Value = serde_json::from_str(candidate)
        .map_err(|e| ParseError::Decode(e.to_string()))?;

    let obj = value.as_object().ok_or(ParseError::Decode("not an object".into()))?;
    // `thought` is recommended but not strictly required — some smaller
    // models drop it. We default to empty rather than fail the parse.
    let thought = obj
        .get("thought")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let action_obj = obj
        .get("action")
        .and_then(|v| v.as_object())
        .ok_or(ParseError::MissingField("action"))?;
    let tool = action_obj
        .get("tool")
        .and_then(|v| v.as_str())
        .ok_or(ParseError::MissingField("action.tool"))?
        .to_string();
    let args = action_obj
        .get("args")
        .cloned()
        .unwrap_or(serde_json::Value::Object(serde_json::Map::new()));

    Ok(ParsedAction { thought, tool, args })
}

/// Find the first balanced `{...}` substring in `s`. Returns the slice
/// pointing at it, or None if no balanced object is found.
fn locate_json_object(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let mut start = None;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    for (i, &b) in bytes.iter().enumerate() {
        if esc { esc = false; continue; }
        if in_str {
            match b {
                b'\\' => esc = true,
                b'"' => in_str = false,
                _ => {}
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => {
                if depth == 0 { start = Some(i); }
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s0) = start {
                        return Some(&s[s0..=i]);
                    }
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use hex_core::ports::inference::mock::MockInferencePort;

    fn echo_action(tool: &str, args: serde_json::Value) -> String {
        serde_json::json!({
            "thought": format!("calling {}", tool),
            "action": { "tool": tool, "args": args }
        }).to_string()
    }

    // ---- parse_action ----

    #[test]
    fn parse_action_happy_path() {
        let p = parse_action(r#"{"thought":"do it","action":{"tool":"repo_read","args":{"path":"x"}}}"#).unwrap();
        assert_eq!(p.thought, "do it");
        assert_eq!(p.tool, "repo_read");
        assert_eq!(p.args["path"], "x");
    }

    #[test]
    fn parse_action_tolerates_leading_prose() {
        let reply = "Sure! Here is my next step:\n{\"thought\":\"x\",\"action\":{\"tool\":\"repo_read\",\"args\":{}}}";
        let p = parse_action(reply).unwrap();
        assert_eq!(p.tool, "repo_read");
    }

    #[test]
    fn parse_action_tolerates_leading_fence_and_trailing_prose() {
        let reply = "```json\n{\"thought\":\"x\",\"action\":{\"tool\":\"repo_read\",\"args\":{}}}\n```\nLet me know if you need anything else.";
        let p = parse_action(reply).unwrap();
        assert_eq!(p.tool, "repo_read");
    }

    #[test]
    fn parse_action_handles_nested_braces_in_args() {
        let reply = r#"{"thought":"x","action":{"tool":"code_patch_propose","args":{"path":"a.rs","mode":"create","content":"fn x() { let m = HashMap::new(); }"}}}"#;
        let p = parse_action(reply).unwrap();
        assert_eq!(p.tool, "code_patch_propose");
        assert_eq!(p.args["mode"], "create");
    }

    #[test]
    fn parse_action_no_json_is_error() {
        assert!(matches!(parse_action("nothing here"), Err(ParseError::NoJsonObject)));
    }

    #[test]
    fn parse_action_missing_action_is_error() {
        let err = parse_action(r#"{"thought":"x"}"#).unwrap_err();
        assert!(matches!(err, ParseError::MissingField("action")));
    }

    #[test]
    fn parse_action_missing_tool_is_error() {
        let err = parse_action(r#"{"thought":"x","action":{"args":{}}}"#).unwrap_err();
        assert!(matches!(err, ParseError::MissingField("action.tool")));
    }

    #[test]
    fn parse_action_args_default_to_empty_object_when_absent() {
        let p = parse_action(r#"{"thought":"x","action":{"tool":"echo"}}"#).unwrap();
        assert!(p.args.is_object());
        assert!(p.args.as_object().unwrap().is_empty());
    }

    // ---- driver ----

    use crate::orchestration::agent_loop::tool::{IAgentTool, Observation, TerminalAction, ToolError};
    use async_trait::async_trait;

    /// Test tool that records its calls + returns canned observations.
    struct ScriptedTool {
        name_: &'static str,
        // Each invoke pops the next entry. Empty → returns Observation::ok("(scripted-default)").
        responses: tokio::sync::Mutex<Vec<Result<Observation, ToolError>>>,
    }

    impl ScriptedTool {
        fn new(name: &'static str, responses: Vec<Result<Observation, ToolError>>) -> Self {
            Self { name_: name, responses: tokio::sync::Mutex::new(responses) }
        }
    }

    #[async_trait]
    impl IAgentTool for ScriptedTool {
        fn name(&self) -> &str { self.name_ }
        fn description(&self) -> &str { "scripted test tool" }
        fn schema(&self) -> serde_json::Value { serde_json::json!({}) }
        async fn invoke(&self, _args: serde_json::Value) -> Result<Observation, ToolError> {
            let mut q = self.responses.lock().await;
            q.pop().unwrap_or_else(|| Ok(Observation::ok("(scripted-default)")))
        }
    }

    fn run_input<'a>(
        task_brief: &'a str,
        tools: Vec<Box<dyn IAgentTool>>,
        max_steps: u32,
        mock: Arc<MockInferencePort>,
    ) -> AgentRunInput<'a> {
        AgentRunInput {
            role: "hex-coder",
            task_brief,
            tools,
            max_steps,
            max_output_tokens: 100_000,
            inference: mock,
            model: "mock".into(),
        }
    }

    #[tokio::test]
    async fn driver_terminates_on_code_patch_propose() {
        // Mock returns ONE reply: a code_patch_propose call. The scripted
        // tool returns Observation::terminal so the driver halts on step 0.
        let mock = Arc::new(MockInferencePort::with_response(echo_action(
            "code_patch_propose",
            serde_json::json!({"path":"hex-cli/tests/foo.rs","mode":"create","content":"fn x(){}"})
        )));
        let tool = Box::new(ScriptedTool::new(
            "code_patch_propose",
            vec![Ok(Observation::terminal(TerminalAction {
                tool: "code_patch_propose".into(),
                path: "hex-cli/tests/foo.rs".into(),
                mode: "create".into(),
                content: "fn x(){}".into(),
            }))]
        )) as Box<dyn IAgentTool>;
        let t = run(run_input("brief", vec![tool], 4, mock)).await;
        assert_eq!(t.terminated_reason, TerminatedReason::TerminalAction);
        assert_eq!(t.steps.len(), 1);
        assert!(t.final_action.is_some());
        assert_eq!(t.final_action.unwrap().path, "hex-cli/tests/foo.rs");
    }

    #[tokio::test]
    async fn driver_terminates_on_max_steps() {
        // Mock returns the same non-terminal action forever; driver should
        // halt with MaxSteps after the budget.
        let mock = Arc::new(MockInferencePort::with_response(echo_action(
            "repo_read",
            serde_json::json!({"path":"docs/x.md"})
        )));
        let tool = Box::new(ScriptedTool::new("repo_read", vec![])) as Box<dyn IAgentTool>;
        let t = run(run_input("brief", vec![tool], 3, mock)).await;
        assert_eq!(t.terminated_reason, TerminatedReason::MaxSteps);
        assert_eq!(t.steps.len(), 3);
        assert!(t.final_action.is_none());
    }

    #[tokio::test]
    async fn driver_terminates_on_unknown_tool() {
        let mock = Arc::new(MockInferencePort::with_response(echo_action(
            "no_such_tool",
            serde_json::json!({})
        )));
        let tool = Box::new(ScriptedTool::new("repo_read", vec![])) as Box<dyn IAgentTool>;
        let t = run(run_input("brief", vec![tool], 4, mock)).await;
        match t.terminated_reason {
            TerminatedReason::UnknownTool { name } => assert_eq!(name, "no_such_tool"),
            other => panic!("expected UnknownTool, got {:?}", other),
        }
        // We still record the step so the operator can see what was asked for.
        assert_eq!(t.steps.len(), 1);
    }

    #[tokio::test]
    async fn driver_terminates_on_parse_exhausted() {
        // Mock returns gibberish on every turn; driver should burn through
        // PARSE_RETRY_BUDGET and halt with ParseExhausted.
        let mock = Arc::new(MockInferencePort::with_response("totally not json"));
        let tool = Box::new(ScriptedTool::new("repo_read", vec![])) as Box<dyn IAgentTool>;
        let t = run(run_input("brief", vec![tool], 10, mock)).await;
        assert_eq!(t.terminated_reason, TerminatedReason::ParseExhausted);
        // PARSE_RETRY_BUDGET error-steps recorded; no real tool was ever called.
        assert_eq!(t.steps.len(), PARSE_RETRY_BUDGET as usize);
    }

    #[tokio::test]
    async fn driver_records_tool_error_and_keeps_going() {
        // Mock keeps asking for the tool. First invoke returns a
        // ToolError::PolicyDenied; the driver records it and goes another
        // turn (so we still hit MaxSteps once the budget expires).
        let mock = Arc::new(MockInferencePort::with_response(echo_action(
            "repo_read",
            serde_json::json!({"path":"/etc/passwd"})
        )));
        let tool = Box::new(ScriptedTool::new(
            "repo_read",
            vec![
                Err(ToolError::PolicyDenied("absolute path rejected".into())),
                Err(ToolError::PolicyDenied("absolute path rejected".into())),
            ]
        )) as Box<dyn IAgentTool>;
        let t = run(run_input("brief", vec![tool], 2, mock)).await;
        assert_eq!(t.terminated_reason, TerminatedReason::MaxSteps);
        assert_eq!(t.steps.len(), 2);
        assert!(t.steps[0].observation.starts_with("error:"));
        assert!(t.steps[0].observation.contains("policy denied"));
    }

    #[tokio::test]
    async fn driver_terminates_on_inference_failure() {
        // Unreachable mock returns InferenceError on every complete().
        let mock = Arc::new(MockInferencePort::unreachable());
        let tool = Box::new(ScriptedTool::new("repo_read", vec![])) as Box<dyn IAgentTool>;
        let t = run(run_input("brief", vec![tool], 5, mock)).await;
        match t.terminated_reason {
            TerminatedReason::InferenceFailed { .. } => {}
            other => panic!("expected InferenceFailed, got {:?}", other),
        }
        // No steps recorded — inference failed before tool dispatch.
        assert!(t.steps.is_empty());
    }

    // locate_json_object guard tests

    #[test]
    fn locate_json_skips_braces_inside_strings() {
        let s = r#"prefix {"k":"value with } inside","action":{"tool":"x"}} trailer"#;
        let extracted = locate_json_object(s).unwrap();
        let v: serde_json::Value = serde_json::from_str(extracted).unwrap();
        assert_eq!(v["k"], "value with } inside");
        assert_eq!(v["action"]["tool"], "x");
    }

    #[test]
    fn locate_json_handles_escaped_quotes() {
        let s = r#"text {"k":"he said \"hi\""} text"#;
        let extracted = locate_json_object(s).unwrap();
        let v: serde_json::Value = serde_json::from_str(extracted).unwrap();
        assert_eq!(v["k"], r#"he said "hi""#);
    }

    #[test]
    fn locate_json_returns_none_for_unbalanced() {
        assert!(locate_json_object("{ no closing").is_none());
        assert!(locate_json_object("nothing at all").is_none());
    }
}
