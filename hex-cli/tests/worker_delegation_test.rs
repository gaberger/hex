/// Integration tests for ADR-2603291900 — Docker Worker First-Class Execution.
///
/// Verifies the three seams of the worker delegation path without requiring
/// a live Docker daemon or nexus instance:
///
/// 1. WorkerResult JSON deserialization — the exact format workers store in hexflo memory
/// 2. Memory key naming conventions — {task_id}:result and {task_id}:step_metadata
/// 3. Filesystem-based objective evaluation — CodeGenerated passes when worker writes files
///
/// The full end-to-end path (Docker spawn → task poll → file commit) is exercised
/// manually via `hex dev start --auto` with Docker available.

use hex_cli::pipeline::objectives::{evaluate_all, Objective};
use hex_cli::pipeline::supervisor::WorkerResult;

// ── WorkerResult serde ────────────────────────────────────────────────────────

/// The worker stores this exact JSON shape in hexflo memory under {task_id}:result.
/// The supervisor deserializes it via `read_worker_result`. Verify round-trip.
#[test]
fn worker_result_deserializes_from_worker_json() {
    let json = serde_json::json!({
        "file_path": "src/main.go",
        "content_len": 512,
        "compile_pass": true,
        "tests_pass": true,
        "test_output": "ok\tall tests pass",
        // extra fields workers may include (must be ignored, not error)
        "model": "claude-sonnet-4-5",
        "cost_usd": 0.0012,
        "tokens": 1200,
    });

    let result: WorkerResult = serde_json::from_value(json).expect("WorkerResult must deserialize");
    assert_eq!(result.file_path, "src/main.go");
    assert_eq!(result.content_len, 512);
    assert!(result.compile_pass);
    assert!(result.tests_pass);
    assert_eq!(result.test_output, "ok\tall tests pass");
}

#[test]
fn worker_result_deserializes_with_compile_failure() {
    let json = serde_json::json!({
        "file_path": "src/handler.go",
        "content_len": 300,
        "compile_pass": false,
        "tests_pass": false,
        "test_output": "",
    });

    let result: WorkerResult = serde_json::from_value(json).expect("WorkerResult must deserialize");
    assert!(!result.compile_pass);
    assert!(!result.tests_pass);
}

// ── Memory key conventions ────────────────────────────────────────────────────

/// The supervisor reads `{task_id}:result` and workers write to the same key.
/// Verify the key format is stable.
#[test]
fn worker_result_memory_key_format() {
    let task_id = "88bb424c-591a-482e-ac4f-55969549b7cf";
    let result_key = format!("{}:result", task_id);
    let metadata_key = format!("{}:step_metadata", task_id);
    let files_key = format!("{}:generated_files", task_id);

    assert_eq!(result_key, "88bb424c-591a-482e-ac4f-55969549b7cf:result");
    assert_eq!(
        metadata_key,
        "88bb424c-591a-482e-ac4f-55969549b7cf:step_metadata"
    );
    assert_eq!(
        files_key,
        "88bb424c-591a-482e-ac4f-55969549b7cf:generated_files"
    );
}

// ── Filesystem-based objective evaluation ─────────────────────────────────────

/// The core guarantee of worker delegation: once the worker writes files to the
/// Docker-mounted output_dir, the supervisor's next evaluate_all call discovers
/// them via filesystem scan and marks CodeGenerated as met.
#[tokio::test]
async fn code_generated_passes_after_worker_writes_go_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output_dir = tmp.path().to_str().unwrap();

    // Simulate worker writing a file to /workspace (Docker bind-mounted to output_dir)
    std::fs::write(
        tmp.path().join("main.go"),
        "package main\n\nimport \"fmt\"\n\nfunc main() { fmt.Println(\"hello\") }\n",
    )
    .unwrap();

    let states = evaluate_all(
        &[Objective::CodeGenerated],
        &[],
        0,
        output_dir,
        "go",
        "http://localhost:5555", // nexus URL — not called for CodeGenerated
    )
    .await;

    let code_gen = states
        .iter()
        .find(|s| s.objective == Objective::CodeGenerated)
        .expect("CodeGenerated must be evaluated");

    assert!(
        code_gen.met,
        "CodeGenerated must pass after worker writes .go files: {}",
        code_gen.detail
    );
}

/// Guard: empty directory must fail CodeGenerated (regression check).
#[tokio::test]
async fn code_generated_fails_on_empty_dir() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output_dir = tmp.path().to_str().unwrap();

    let states = evaluate_all(
        &[Objective::CodeGenerated],
        &[],
        0,
        output_dir,
        "go",
        "http://localhost:5555",
    )
    .await;

    let code_gen = states
        .iter()
        .find(|s| s.objective == Objective::CodeGenerated)
        .expect("CodeGenerated must be evaluated");

    assert!(
        !code_gen.met,
        "CodeGenerated must fail on empty dir"
    );
}

/// TypeScript worker writes to src/ — verify CodeGenerated passes for TS projects.
#[tokio::test]
async fn code_generated_passes_after_worker_writes_ts_files() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let output_dir = tmp.path().to_str().unwrap();

    // TS tier-0 expects files in src/core/domain (hex hexagonal layout)
    let domain = tmp.path().join("src").join("core").join("domain");
    std::fs::create_dir_all(&domain).unwrap();
    std::fs::write(
        domain.join("value-objects.ts"),
        "export type RaceId = string;\n",
    )
    .unwrap();

    let states = evaluate_all(
        &[Objective::CodeGenerated],
        &[],
        0,
        output_dir,
        "typescript",
        "http://localhost:5555",
    )
    .await;

    let code_gen = states
        .iter()
        .find(|s| s.objective == Objective::CodeGenerated)
        .expect("CodeGenerated must be evaluated");

    assert!(
        code_gen.met,
        "CodeGenerated must pass after worker writes .ts files: {}",
        code_gen.detail
    );
}
