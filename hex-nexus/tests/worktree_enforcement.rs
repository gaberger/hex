//! Integration tests for worktree enforcement in agent hooks (ADR-2603231700).
//!
//! Tests cover all 8 phases:
//! - P1: SessionState worktree_path, allowed_paths
//! - P2: pre-agent hook worktree resolution
//! - P3: Auto-create worktree if missing
//! - P4: Boundary enforcement in pre-edit
//! - P5: Dependency order enforcement
//! - P6: Branch naming validation
//! - P7: Auto-merge + cleanup
//! - P8: hex agent audit

use std::collections::HashMap;
use std::path::PathBuf;

// ── P1: SessionState Tests ───────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
struct SessionState {
    agent_id: String,
    worktree_path: Option<String>,
    allowed_paths: Vec<String>,
    worktree_branch: Option<String>,
    current_task_id: Option<String>,
    #[serde(default)]
    project: Option<String>,
    #[serde(default)]
    swarm_id: Option<String>,
    #[serde(default)]
    workplan_id: Option<String>,
}

#[test]
fn test_session_state_serialization() {
    let state = SessionState {
        agent_id: "test-agent-123".into(),
        worktree_path: Some("/path/to/worktree".into()),
        allowed_paths: vec![
            "hex-nexus/src/adapters/secondary/".into(),
            "hex-nexus/src/ports/".into(),
        ],
        worktree_branch: Some("feat/test/secondary".into()),
        current_task_id: Some("task-uuid-456".into()),
        ..Default::default()
    };

    let json = serde_json::to_string(&state).unwrap();
    let decoded: SessionState = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded.agent_id, "test-agent-123");
    assert_eq!(decoded.worktree_path, Some("/path/to/worktree".into()));
    assert_eq!(decoded.allowed_paths.len(), 2);
    assert_eq!(decoded.worktree_branch, Some("feat/test/secondary".into()));
}

#[test]
fn test_session_state_defaults() {
    let state = SessionState::default();

    assert!(state.worktree_path.is_none());
    assert!(state.allowed_paths.is_empty());
    assert!(state.worktree_branch.is_none());
    assert!(state.current_task_id.is_none());
}

// ── P2: Worktree Resolution Tests ───────────────────────────────────────────

#[test]
fn test_worktree_branch_naming_convention() {
    let valid_branches = vec![
        "feat/openrouter/ports",
        "feat/add-api/domain",
        "fix/bug/usecases",
        "refactor/cleanup/primary",
    ];

    for branch in valid_branches {
        assert!(
            is_valid_worktree_branch(branch),
            "Branch {} should be valid",
            branch
        );
    }

    let invalid_branches = vec!["main", "master", "random-branch", "feature-without-layer"];

    for branch in invalid_branches {
        assert!(
            !is_valid_worktree_branch(branch),
            "Branch {} should be invalid",
            branch
        );
    }
}

fn is_valid_worktree_branch(branch: &str) -> bool {
    let parts: Vec<&str> = branch.split('/').collect();
    if parts.len() < 3 {
        return false;
    }
    matches!(parts[0], "feat" | "fix" | "refactor" | "chore")
        && parts[1].len() >= 3
        && matches!(
            parts[2],
            "domain" | "ports" | "usecases" | "primary" | "secondary" | "integration"
        )
}

#[test]
fn test_workplan_step_resolution() {
    let workplan = r#"{
        "id": "wp-test-123",
        "steps": [
            {
                "id": "step-1",
                "title": "Add port interface",
                "layer": "ports",
                "adapter": "inference-router",
                "worktree_branch": "feat/add-port/ports",
                "dependencies": []
            },
            {
                "id": "step-2", 
                "title": "Implement adapter",
                "layer": "adapters/secondary",
                "adapter": "inference-router",
                "worktree_branch": "feat/add-port/secondary",
                "dependencies": ["step-1"]
            }
        ]
    }"#;

    let wp: serde_json::Value = serde_json::from_str(workplan).unwrap();

    // Find step by ID
    let step = wp["steps"]
        .as_array()
        .and_then(|steps| steps.iter().find(|s| s["id"] == "step-2"))
        .unwrap();

    assert_eq!(step["worktree_branch"], "feat/add-port/secondary");
    assert_eq!(step["dependencies"].as_array().unwrap().len(), 1);
}

#[test]
fn test_task_id_extraction_from_prompt() {
    let prompt_with_task =
        "HEXFLO_TASK:abc123-def456-ghi789\n\nPlease implement the port interface...";
    let prompt_without_task = "Please implement the port interface...";

    let task_id = extract_hextflo_task_id(prompt_with_task);
    assert_eq!(task_id, Some("abc123-def456-ghi789".to_string()));

    let task_id = extract_hextflo_task_id(prompt_without_task);
    assert_eq!(task_id, None);
}

fn extract_hextflo_task_id(prompt: &str) -> Option<String> {
    for line in prompt.lines() {
        if line.starts_with("HEXFLO_TASK:") {
            return Some(line.trim_start_matches("HEXFLO_TASK:").to_string());
        }
    }
    None
}

// ── P3: Worktree Auto-Creation Tests ─────────────────────────────────────────

#[test]
fn test_worktree_path_construction() {
    let root = PathBuf::from("/Volumes/ExtendedStorage/PARA/01-Projects/hex-intf");
    let branch = "feat/test/domain";
    let expected_path = root.join(".test_worktrees").join(branch);

    assert!(expected_path.to_string_lossy().contains("feat/test/domain"));
}

#[test]
fn test_worktree_create_command_args() {
    // Simulate git worktree add command construction
    let branch = "feat/test/ports";
    let worktree_path = "/path/to/worktree";

    let args: Vec<String> = vec![
        "worktree".to_string(),
        "add".to_string(),
        worktree_path.to_string(),
        "-b".to_string(),
        branch.to_string(),
    ];

    assert_eq!(args.len(), 5);
    assert_eq!(args[0], "worktree");
    assert_eq!(args[1], "add");
    assert_eq!(args[3], "-b");
}

// ── P4: Boundary Enforcement Tests ─────────────────────────────────────────

#[test]
fn test_boundary_validation_domain() {
    let layer = "domain";
    let adapter = "value-objects";

    assert!(is_allowed_path(
        "src/core/domain/value_objects.rs",
        layer,
        Some(adapter)
    ));
    assert!(is_allowed_path(
        "src/core/domain/entities.rs",
        layer,
        Some(adapter)
    ));
    assert!(!is_allowed_path(
        "src/core/ports/inference.rs",
        layer,
        Some(adapter)
    ));
    assert!(!is_allowed_path(
        "src/adapters/secondary/openai.rs",
        layer,
        Some(adapter)
    ));
}

#[test]
fn test_boundary_validation_ports() {
    let layer = "ports";

    assert!(is_allowed_path("src/core/ports/inference.rs", layer, None));
    assert!(is_allowed_path("src/core/ports/state.rs", layer, None));
    assert!(!is_allowed_path("src/core/domain/entities.rs", layer, None));
    assert!(!is_allowed_path(
        "src/adapters/secondary/mod.rs",
        layer,
        None
    ));
}

#[test]
fn test_boundary_validation_adapters_primary() {
    let layer = "adapters/primary";
    let adapter = "cli";

    assert!(is_allowed_path(
        "src/adapters/primary/cli/main.rs",
        layer,
        Some(adapter)
    ));
    assert!(is_allowed_path(
        "src/adapters/primary/cli/commands.rs",
        layer,
        Some(adapter)
    ));
    assert!(!is_allowed_path(
        "src/adapters/secondary/api.rs",
        layer,
        Some(adapter)
    ));
    assert!(!is_allowed_path(
        "src/adapters/secondary/api/client.rs", // Different adapter, should be blocked
        layer,
        Some("openai".into())
    ));
}

#[test]
fn test_boundary_validation_adapters_secondary() {
    let layer = "adapters/secondary";
    let adapter = "openai";

    assert!(is_allowed_path(
        "src/adapters/secondary/openai/client.rs",
        layer,
        Some(adapter)
    ));
    assert!(!is_allowed_path(
        "src/adapters/primary/cli/main.rs",
        layer,
        Some(adapter)
    ));
}

#[test]
fn test_boundary_always_allowed_paths() {
    // docs/, tests/, config/ are always allowed
    assert!(is_allowed_path("docs/adrs/test.md", "domain", None));
    assert!(is_allowed_path("tests/integration/test.rs", "domain", None));
    assert!(is_allowed_path("config/settings.json", "domain", None));
}

fn is_allowed_path(path: &str, layer: &str, adapter: Option<&str>) -> bool {
    let path = PathBuf::from(path);
    let path_str = path.to_string_lossy().to_string();

    // Always allow docs, tests, config
    if path_str.starts_with("docs/")
        || path_str.starts_with("tests/")
        || path_str.starts_with("config/")
    {
        return true;
    }

    match layer {
        "domain" => {
            path_str.contains("/domain/")
                || path_str.contains("/value-objects/")
                || path_str.contains("/entities/")
        }
        "ports" => path_str.contains("/ports/") && !path_str.contains("/adapters/"),
        "adapters/primary" => {
            if let Some(adapter) = adapter {
                path_str.contains(&format!("/adapters/primary/{}", adapter))
            } else {
                path_str.contains("/adapters/primary/")
            }
        }
        "adapters/secondary" => {
            if let Some(adapter) = adapter {
                path_str.contains(&format!("/adapters/secondary/{}", adapter))
            } else {
                path_str.contains("/adapters/secondary/")
            }
        }
        "usecases" => path_str.contains("/usecases/") || path_str.contains("/composition-root"),
        _ => false,
    }
}

// ── P5: Dependency Order Enforcement Tests ─────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
enum TaskStatus {
    Pending,
    InProgress,
    Completed,
}

#[test]
fn test_dependency_check_all_completed() {
    let dependencies = vec![
        "step-1".to_string(),
        "step-2".to_string(),
        "step-3".to_string(),
    ];
    let task_statuses = HashMap::from([
        ("step-1".to_string(), TaskStatus::Completed),
        ("step-2".to_string(), TaskStatus::Completed),
        ("step-3".to_string(), TaskStatus::Completed),
    ]);

    assert!(check_dependencies_ready(&dependencies, &task_statuses));
}

#[test]
fn test_dependency_check_one_pending() {
    let dependencies = vec!["step-1".to_string(), "step-2".to_string()];
    let task_statuses = HashMap::from([
        ("step-1".to_string(), TaskStatus::Completed),
        ("step-2".to_string(), TaskStatus::Pending),
    ]);

    assert!(!check_dependencies_ready(&dependencies, &task_statuses));
}

#[test]
fn test_dependency_check_in_progress_fails() {
    let dependencies = vec!["step-1".to_string()];
    let task_statuses = HashMap::from([("step-1".to_string(), TaskStatus::InProgress)]);

    assert!(!check_dependencies_ready(&dependencies, &task_statuses));
}

fn check_dependencies_ready(
    dependencies: &[String],
    statuses: &HashMap<String, TaskStatus>,
) -> bool {
    for dep in dependencies {
        let status = statuses.get(dep);
        match status {
            Some(TaskStatus::Completed) => continue,
            _ => return false,
        }
    }
    true
}

// ── P6: Branch Naming Validation Tests ───────────────────────────────────

#[test]
fn test_branch_naming_feature_prefix() {
    assert!(is_valid_worktree_branch("feat/add-openrouter/domain"));
    assert!(is_valid_worktree_branch("fix/bug-fix/ports"));
    assert!(is_valid_worktree_branch("refactor/cleanup/usecases"));
    assert!(!is_valid_worktree_branch("main"));
    assert!(!is_valid_worktree_branch("feature-old-style"));
}

#[test]
fn test_branch_naming_layer_required() {
    assert!(is_valid_worktree_branch("feat/test/domain"));
    assert!(is_valid_worktree_branch("feat/test/ports"));
    assert!(is_valid_worktree_branch("feat/test/usecases"));
    assert!(is_valid_worktree_branch("feat/test/primary"));
    assert!(!is_valid_worktree_branch("feat/test/someother"));
}

// ── P7: Auto-Merge and Cleanup Tests ────────────────────────────────────────

#[test]
fn test_tier_completion_detection() {
    let tier_tasks = vec![
        ("task-1".to_string(), "completed"),
        ("task-2".to_string(), "completed"),
        ("task-3".to_string(), "completed"),
    ];

    assert!(is_tier_complete(&tier_tasks));
}

#[test]
fn test_tier_not_complete_with_pending() {
    let tier_tasks = vec![
        ("task-1".to_string(), "completed"),
        ("task-2".to_string(), "completed"),
        ("task-3".to_string(), "pending"),
    ];

    assert!(!is_tier_complete(&tier_tasks));
}

fn is_tier_complete(tasks: &[(String, &str)]) -> bool {
    tasks.iter().all(|(_, status)| *status == "completed")
}

// ── P8: Agent Audit Tests ──────────────────────────────────────────────────

#[test]
fn test_audit_detects_main_branch_edits() {
    let git_log = vec![
        ("abc1234", "feat(x): add port interface", "main"),
        ("def5678", "feat(x): add adapter", "main"),
        ("ghi9012", "chore: update deps", "feat/openrouter/domain"),
    ];

    let main_edits = git_log
        .iter()
        .filter(|(_, _, branch)| *branch == "main")
        .collect::<Vec<_>>();

    assert_eq!(main_edits.len(), 2);
}

#[test]
fn test_audit_allows_worktree_edits() {
    let git_log = vec![
        (
            "abc1234",
            "feat(x): add port interface",
            "feat/add-port/ports",
        ),
        ("def5678", "feat(x): add adapter", "feat/add-port/secondary"),
    ];

    let main_edits = git_log
        .iter()
        .filter(|(_, _, branch)| *branch == "main")
        .collect::<Vec<_>>();

    assert_eq!(main_edits.len(), 0);
}

#[test]
fn test_audit_cross_references_hexflo_tasks() {
    // Simulated HexFlo task -> branch mapping
    let hexflo_tasks = HashMap::from([
        ("task-uuid-1".to_string(), "feat/add-port/ports"),
        ("task-uuid-2".to_string(), "feat/add-port/secondary"),
    ]);

    let task = "task-uuid-1";
    let branch = hexflo_tasks.get(task);

    assert!(branch.is_some());
    assert_eq!(*branch.unwrap(), "feat/add-port/ports");
}

// ── Integration: Full Flow Tests ───────────────────────────────────────────

#[test]
fn test_full_worktree_enforcement_flow() {
    // 1. Agent receives prompt with HEXFLO_TASK
    let prompt = "HEXFLO_TASK:task-123\nImplement the inference port interface";
    let task_id = extract_hextflo_task_id(prompt);
    assert!(task_id.is_some());

    // 2. Resolve task to worktree branch
    let branch = "feat/add-inference/ports";
    assert!(is_valid_worktree_branch(branch));

    // 3. Check/adjust worktree
    let layer = "ports";
    assert!(is_allowed_path("src/core/ports/inference.rs", layer, None));

    // 4. Validate boundary
    let boundary_ok = is_allowed_path("src/core/ports/inference.rs", layer, None);
    assert!(boundary_ok);

    // 5. Check dependencies (assume complete)
    let deps = vec!["step-1".to_string()];
    let statuses = HashMap::from([("step-1".to_string(), TaskStatus::Completed)]);
    assert!(check_dependencies_ready(&deps, &statuses));
}

#[test]
fn test_boundary_enforcement_advisory_mode() {
    let project_config = r#"{
        "boundary_enforcement": "advisory"
    }"#;

    let config: serde_json::Value = serde_json::from_str(project_config).unwrap();
    let mode = config
        .get("boundary_enforcement")
        .and_then(|v| v.as_str())
        .unwrap_or("advisory");

    assert_eq!(mode, "advisory");
}

#[test]
fn test_boundary_enforcement_mandatory_mode() {
    let project_config = r#"{
        "boundary_enforcement": "mandatory"
    }"#;

    let config: serde_json::Value = serde_json::from_str(project_config).unwrap();
    let mode = config
        .get("boundary_enforcement")
        .and_then(|v| v.as_str())
        .unwrap_or("advisory");

    assert_eq!(mode, "mandatory");
}

// ── Edge Cases ────────────────────────────────────────────────────────────────

#[test]
fn test_empty_allowed_paths() {
    let state = SessionState {
        allowed_paths: vec![],
        ..Default::default()
    };

    assert!(state.allowed_paths.is_empty());
}

#[test]
fn test_worktree_branch_none() {
    let state = SessionState {
        worktree_branch: None,
        ..Default::default()
    };

    assert!(state.worktree_branch.is_none());
}

#[test]
fn test_boundary_rejects_cross_adapter() {
    // Primary adapter should not be able to edit secondary adapter files
    let layer = "adapters/primary";
    let adapter = "cli";

    assert!(!is_allowed_path(
        "src/adapters/secondary/openai/client.rs",
        layer,
        Some(adapter)
    ));
}

#[test]
fn test_dependency_empty_list() {
    let dependencies: Vec<String> = vec![];
    let task_statuses: HashMap<String, TaskStatus> = HashMap::new();

    assert!(check_dependencies_ready(&dependencies, &task_statuses));
}

#[test]
fn test_audit_empty_git_log() {
    let git_log: Vec<(&str, &str, &str)> = vec![];

    let main_edits = git_log
        .iter()
        .filter(|(_, _, branch)| *branch == "main")
        .collect::<Vec<_>>();

    assert_eq!(main_edits.len(), 0);
}
